use super::{TuiElement, TuiRenderOutput};
use crate::elements::tui::{TuiBuffer, TuiConstraint, TuiRect, TuiSize};

#[test]
fn unit_element_is_inert() {
    let mut element = ();
    assert_eq!(
        element.layout(TuiConstraint::tight(TuiSize::new(4, 2))),
        TuiSize::ZERO,
    );
    assert_eq!(element.desired_height(10), 0);
    assert_eq!(
        TuiElement::cursor_position(&(), TuiRect::new(0, 0, 4, 2)),
        None
    );

    // Painting a `()` leaves the buffer untouched.
    let mut buffer = TuiBuffer::new(TuiSize::new(2, 1));
    element.render(TuiRect::new(0, 0, 2, 1), &mut buffer);
    assert_eq!(buffer.to_lines(), vec!["  "]);
}

#[test]
fn unit_element_is_boxable_as_render_output() {
    let boxed: TuiRenderOutput = Box::new(());
    assert_eq!(boxed.desired_height(3), 0);
}
