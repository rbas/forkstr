use forkstr::{
    model::{CommandId, PaneLayout},
    terminal::{Focus, pane_rects},
};
use ratatui::layout::Rect;

#[test]
fn completed_panes_keep_selection_but_lose_input_focus() {
    let old = CommandId {
        stage: 0,
        command: 0,
    };
    let new = CommandId {
        stage: 0,
        command: 1,
    };
    let mut focus = Focus::default();
    focus.reconcile(&[old], &[old]);
    focus.enter(&[old]);
    assert_eq!(focus.input_target(), Some(old));
    focus.reconcile(&[old, new], &[new]);
    assert_eq!(focus.input_target(), None);
    focus.enter(&[new]);
    assert_eq!(focus.input_target(), None);
    focus.cycle(&[old, new], false);
    focus.enter(&[new]);
    assert_eq!(focus.input_target(), Some(new));
    let next_stage = CommandId {
        stage: 1,
        command: 0,
    };
    focus.reconcile(&[next_stage], &[next_stage]);
    assert_eq!(focus.input_target(), None);
    focus.enter(&[next_stage]);
    assert_eq!(focus.input_target(), Some(next_stage));
}

#[test]
fn layouts_fit_and_small_windows_fall_back_to_one_pane() {
    for layout in [
        PaneLayout::Auto,
        PaneLayout::Horizontal,
        PaneLayout::Vertical,
    ] {
        let area = Rect::new(0, 3, 120, 40);
        let panes = pane_rects(area, 4, layout);
        assert_eq!(panes.len(), 4);
        for pane in panes {
            assert!(pane.right() <= area.right());
            assert!(pane.bottom() <= area.bottom());
        }
        assert_eq!(pane_rects(Rect::new(0, 0, 30, 10), 10, layout).len(), 1);
    }
}
