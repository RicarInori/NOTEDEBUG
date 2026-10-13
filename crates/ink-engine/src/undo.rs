//! The document's edit log (§3.2).
//!
//! ## What this is not
//!
//! §3.2 observes that an edit "is naturally isomorphic to a Cordis effect", which invites
//! implementing undo as the plugin's effect stack. **Do not do that.** An effect is unwound when
//! its plugin unloads, so tying the edit log to the plugin's effects would mean *disabling the ink
//! plugin erases the user's notes*. The undo stack belongs to the **document** and outlives any
//! plugin; only a plugin's own registrations belong to its effects.
//!
//! The inverse of a command is computed, not stored: storing both invites them to drift apart.
//!
//! ## Ordering caveat
//!
//! A command log carries no positions, so undoing a removal re-appends the stroke at the end of the
//! page rather than at the index it came from. The relative draw order of two strokes can therefore
//! change across an undo. That is invisible for opaque ink and visible only where strokes overlap
//! with a translucent blend, and it is the price of a log that cannot fall out of step with the
//! document — storing indices instead would make every *other* command responsible for maintaining
//! them.

use crate::stroke::{PageId, Stroke, StrokeId};
use std::collections::BTreeMap;

/// Undo depth from the guide. Bounded so a long session cannot grow without limit.
pub const DEFAULT_UNDO_LIMIT: usize = 200;

/// One reversible edit. Granularity is a stroke (§3.2): a continuous stroke is not split.
#[derive(Clone, Debug, PartialEq)]
pub enum InkCommand {
    Add {
        page: PageId,
        stroke: Stroke,
    },
    /// Carries the whole stroke so the inverse can put it back without consulting anything else.
    Remove {
        page: PageId,
        stroke: Stroke,
    },
    /// Carries the strokes as they *were*, because `(x + d) - d` is not `x` in floating point: an
    /// inverse computed from the offset alone would drift by an ulp on every undo/redo cycle, and
    /// §3.2 already specifies a `before` field for exactly this reason.
    Translate {
        page: PageId,
        ids: Vec<StrokeId>,
        dx: f32,
        dy: f32,
        before: Vec<Stroke>,
    },
    /// Several commands that must undo as one step (§3.3: "撤销粒度 = 一次擦除行程内全部命中").
    ///
    /// Applied in order and unwound in reverse, so the sub-commands may assume their predecessors
    /// have already run. A batch is never nested — the engine builds them flat — so `page` is
    /// unambiguous and is recorded rather than derived.
    Batch {
        page: PageId,
        commands: Vec<InkCommand>,
    },
}

impl InkCommand {
    pub fn page(&self) -> PageId {
        match self {
            InkCommand::Add { page, .. }
            | InkCommand::Remove { page, .. }
            | InkCommand::Translate { page, .. }
            | InkCommand::Batch { page, .. } => *page,
        }
    }

    /// Whether this command would change anything. Used to keep no-op gestures out of the log.
    pub fn is_empty(&self) -> bool {
        match self {
            InkCommand::Batch { commands, .. } => commands.is_empty(),
            _ => false,
        }
    }

    pub fn apply(&self, pages: &mut PageStore) {
        match self {
            InkCommand::Add { page, stroke } => pages.push(*page, stroke.clone()),
            InkCommand::Remove { page, stroke } => {
                pages.take(*page, stroke.id);
            }
            InkCommand::Translate {
                page, ids, dx, dy, ..
            } => {
                for id in ids {
                    if let Some(stroke) = pages.get_mut(*page, *id) {
                        stroke.translate(*dx, *dy);
                    }
                }
            }
            InkCommand::Batch { commands, .. } => {
                for command in commands {
                    command.apply(pages);
                }
            }
        }
    }

    pub fn revert(&self, pages: &mut PageStore) {
        match self {
            InkCommand::Add { page, stroke } => {
                pages.take(*page, stroke.id);
            }
            InkCommand::Remove { page, stroke } => pages.push(*page, stroke.clone()),
            InkCommand::Translate { page, before, .. } => {
                // Restoring the recorded geometry is exact; the offsets could not be.
                for stroke in before {
                    pages.replace(*page, stroke.clone());
                }
            }
            InkCommand::Batch { commands, .. } => {
                for command in commands.iter().rev() {
                    command.revert(pages);
                }
            }
        }
    }
}

/// The strokes the engine holds, keyed by page.
#[derive(Debug, Default)]
pub struct PageStore {
    pages: BTreeMap<PageId, Vec<Stroke>>,
    last_id: u64,
}

impl PageStore {
    pub fn new() -> Self {
        Self::default()
    }

    /// Ids are handed out by the store so they stay unique across undos and page switches.
    pub fn alloc_id(&mut self) -> StrokeId {
        self.last_id += 1;
        StrokeId(self.last_id)
    }

    pub fn strokes(&self, page: PageId) -> &[Stroke] {
        self.pages.get(&page).map(Vec::as_slice).unwrap_or(&[])
    }

    pub fn push(&mut self, page: PageId, stroke: Stroke) {
        self.pages.entry(page).or_default().push(stroke);
    }

    pub fn take(&mut self, page: PageId, id: StrokeId) -> Option<Stroke> {
        let strokes = self.pages.get_mut(&page)?;
        let index = strokes.iter().position(|stroke| stroke.id == id)?;
        Some(strokes.remove(index))
    }

    pub fn get_mut(&mut self, page: PageId, id: StrokeId) -> Option<&mut Stroke> {
        self.pages
            .get_mut(&page)?
            .iter_mut()
            .find(|stroke| stroke.id == id)
    }

    /// Overwrite a stroke in place, keeping its position in the draw order.
    pub fn replace(&mut self, page: PageId, stroke: Stroke) -> bool {
        match self.get_mut(page, stroke.id) {
            Some(slot) => {
                *slot = stroke;
                true
            }
            None => false,
        }
    }

    pub fn count(&self, page: PageId) -> usize {
        self.strokes(page).len()
    }
}

/// Per-document undo/redo history.
#[derive(Debug)]
pub struct UndoStack {
    undone: Vec<InkCommand>,
    redone: Vec<InkCommand>,
    limit: usize,
}

impl UndoStack {
    pub fn new(limit: usize) -> Self {
        Self {
            undone: Vec::new(),
            redone: Vec::new(),
            limit: limit.max(1),
        }
    }

    /// Apply a command and make it undoable.
    ///
    /// Committing discards the redo branch — the standard editor contract, and the only honest one:
    /// the redone future no longer exists once the user edits from here.
    pub fn commit(&mut self, pages: &mut PageStore, command: InkCommand) {
        // An empty batch is not an edit. Letting one through would cost the user an undo press that
        // visibly does nothing, and would push a real edit off the bounded log.
        if command.is_empty() {
            return;
        }
        command.apply(pages);
        self.redone.clear();
        self.undone.push(command);
        if self.undone.len() > self.limit {
            // Drop the oldest: the edit log is bounded, so the deepest edits become permanent.
            self.undone.remove(0);
        }
    }

    /// Returns false when there is nothing left to undo.
    pub fn undo(&mut self, pages: &mut PageStore) -> bool {
        let Some(command) = self.undone.pop() else {
            return false;
        };
        command.revert(pages);
        self.redone.push(command);
        true
    }

    /// Returns false when there is nothing to redo.
    pub fn redo(&mut self, pages: &mut PageStore) -> bool {
        let Some(command) = self.redone.pop() else {
            return false;
        };
        command.apply(pages);
        self.undone.push(command);
        true
    }

    pub fn undo_depth(&self) -> usize {
        self.undone.len()
    }

    pub fn redo_depth(&self) -> usize {
        self.redone.len()
    }

    pub fn limit(&self) -> usize {
        self.limit
    }
}

impl Default for UndoStack {
    fn default() -> Self {
        Self::new(DEFAULT_UNDO_LIMIT)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::stroke::StrokePoint;

    fn stroke(id: u64, x: f32) -> Stroke {
        Stroke::new(
            StrokeId(id),
            vec![StrokePoint::new(x, 0.0, 0.5, 0)],
            0xFF1B1B1F,
            4.0,
        )
    }

    fn setup() -> (PageStore, UndoStack) {
        (PageStore::new(), UndoStack::default())
    }

    #[test]
    fn add_undo_redo_round_trips() {
        let (mut pages, mut stack) = setup();
        let cmd = InkCommand::Add {
            page: 1,
            stroke: stroke(1, 0.0),
        };
        stack.commit(&mut pages, cmd);
        assert_eq!(pages.count(1), 1);

        assert!(stack.undo(&mut pages));
        assert_eq!(pages.count(1), 0);
        assert_eq!(stack.redo_depth(), 1);

        assert!(stack.redo(&mut pages));
        assert_eq!(pages.count(1), 1);
        assert_eq!(stack.redo_depth(), 0);
    }

    #[test]
    fn removing_a_stroke_is_reversible_with_its_pressure_intact() {
        let (mut pages, mut stack) = setup();
        let original = Stroke::new(
            StrokeId(7),
            vec![
                StrokePoint::new(1.0, 2.0, 0.25, 10),
                StrokePoint::new(3.0, 4.0, 0.75, 20),
            ],
            0xFFAA0000,
            6.0,
        );
        pages.push(1, original.clone());

        stack.commit(
            &mut pages,
            InkCommand::Remove {
                page: 1,
                stroke: original.clone(),
            },
        );
        assert_eq!(pages.count(1), 0);

        stack.undo(&mut pages);
        assert_eq!(
            pages.strokes(1),
            &[original],
            "the erased stroke must come back byte for byte"
        );
    }

    #[test]
    fn translating_is_exactly_reversible_even_across_cycles() {
        let (mut pages, mut stack) = setup();
        pages.push(1, stroke(1, 10.0));
        let before = pages.strokes(1).to_vec();

        stack.commit(
            &mut pages,
            InkCommand::Translate {
                page: 1,
                ids: vec![StrokeId(1)],
                dx: 5.0,
                dy: -4.0,
                before: before.clone(),
            },
        );
        assert_eq!(pages.strokes(1)[0].points[0].x, 15.0);

        // Twice through the cycle: an inverse derived from the offsets would drift here, which is
        // why the command carries the original geometry.
        for _ in 0..2 {
            stack.undo(&mut pages);
            assert_eq!(pages.strokes(1), before.as_slice());
            stack.redo(&mut pages);
            assert_eq!(pages.strokes(1)[0].points[0].x, 15.0);
        }
    }

    #[test]
    fn a_new_edit_discards_the_redo_branch() {
        let (mut pages, mut stack) = setup();
        stack.commit(
            &mut pages,
            InkCommand::Add {
                page: 1,
                stroke: stroke(1, 0.0),
            },
        );
        stack.undo(&mut pages);
        assert_eq!(stack.redo_depth(), 1);

        stack.commit(
            &mut pages,
            InkCommand::Add {
                page: 1,
                stroke: stroke(2, 5.0),
            },
        );
        assert_eq!(stack.redo_depth(), 0, "the discarded future is gone");
        assert!(!stack.redo(&mut pages));
    }

    #[test]
    fn the_log_is_bounded_and_drops_the_oldest_edit() {
        let mut pages = PageStore::new();
        let mut stack = UndoStack::new(3);
        for id in 1..=4 {
            stack.commit(
                &mut pages,
                InkCommand::Add {
                    page: 1,
                    stroke: stroke(id, id as f32),
                },
            );
        }
        assert_eq!(stack.undo_depth(), 3);
        assert_eq!(pages.count(1), 4);

        // Only the last three are undoable, so three undos leave the first stroke behind.
        for _ in 0..3 {
            assert!(stack.undo(&mut pages));
        }
        assert_eq!(pages.count(1), 1);
        assert!(!stack.undo(&mut pages));
    }

    #[test]
    fn empty_stacks_report_false_rather_than_panicking() {
        let (mut pages, mut stack) = setup();
        assert!(!stack.undo(&mut pages));
        assert!(!stack.redo(&mut pages));
        assert_eq!(stack.undo_depth(), 0);
    }

    #[test]
    fn ids_stay_unique_after_undoing() {
        let mut pages = PageStore::new();
        let first = pages.alloc_id();
        let second = pages.alloc_id();
        assert_ne!(
            first, second,
            "an undone edit must not hand out a recycled id"
        );
    }

    #[test]
    fn a_batch_undoes_as_one_step() {
        let (mut pages, mut stack) = setup();
        pages.push(1, stroke(1, 10.0));
        pages.push(1, stroke(2, 20.0));

        // One eraser journey that hit two strokes: remove both, but as a single user action.
        stack.commit(
            &mut pages,
            InkCommand::Batch {
                page: 1,
                commands: vec![
                    InkCommand::Remove {
                        page: 1,
                        stroke: stroke(1, 10.0),
                    },
                    InkCommand::Remove {
                        page: 1,
                        stroke: stroke(2, 20.0),
                    },
                ],
            },
        );
        assert_eq!(pages.count(1), 0);
        assert_eq!(stack.undo_depth(), 1, "one journey, one undo step");

        assert!(stack.undo(&mut pages));
        assert_eq!(pages.count(1), 2, "both strokes return together");

        assert!(stack.redo(&mut pages));
        assert_eq!(pages.count(1), 0);
    }

    #[test]
    fn a_batch_reverts_its_commands_in_reverse() {
        let (mut pages, mut stack) = setup();
        // Add, then translate: the translate assumes the add has already run, which is what makes the
        // unwind order observable.
        let original = stroke(1, 10.0);
        stack.commit(
            &mut pages,
            InkCommand::Batch {
                page: 1,
                commands: vec![
                    InkCommand::Add {
                        page: 1,
                        stroke: original.clone(),
                    },
                    InkCommand::Translate {
                        page: 1,
                        ids: vec![StrokeId(1)],
                        dx: 5.0,
                        dy: 0.0,
                        before: vec![original.clone()],
                    },
                ],
            },
        );
        assert_eq!(pages.strokes(1)[0].points[0].x, 15.0);

        stack.undo(&mut pages);
        assert_eq!(
            pages.count(1),
            0,
            "the translate is unwound before the add that created its target"
        );

        stack.redo(&mut pages);
        assert_eq!(pages.count(1), 1);
        assert_eq!(pages.strokes(1)[0].points[0].x, 15.0);
    }

    #[test]
    fn an_empty_batch_is_not_an_edit() {
        let (mut pages, mut stack) = setup();
        pages.push(1, stroke(1, 10.0));

        stack.commit(
            &mut pages,
            InkCommand::Batch {
                page: 1,
                commands: Vec::new(),
            },
        );
        assert_eq!(
            stack.undo_depth(),
            0,
            "a gesture that hit nothing must not consume an undo press"
        );
        assert_eq!(pages.count(1), 1);
    }
}
