//! A tiny element wrapper that renders its child with a scaled `rem` size.
//!
//! GPUI's code-editor input derives both its font size *and* line height from
//! the window `rem` size, so the only correct way to "zoom" it (without text
//! overlapping a fixed line height) is to scale `rem` for that subtree. We can't
//! touch `rem` during layout, but `with_rem_size` is valid during prepaint/paint
//! — which is exactly when the editor shapes and positions its text.

use gpui::{
    AnyElement, App, Bounds, Element, GlobalElementId, InspectorElementId, IntoElement, LayoutId,
    Pixels, Window,
};

pub struct RemScaled {
    rem: Pixels,
    child: AnyElement,
}

impl RemScaled {
    pub fn new(rem: Pixels, child: impl IntoElement) -> Self {
        Self {
            rem,
            child: child.into_any_element(),
        }
    }
}

impl IntoElement for RemScaled {
    type Element = Self;
    fn into_element(self) -> Self::Element {
        self
    }
}

impl Element for RemScaled {
    type RequestLayoutState = ();
    type PrepaintState = ();

    fn id(&self) -> Option<gpui::ElementId> {
        None
    }

    fn source_location(&self) -> Option<&'static std::panic::Location<'static>> {
        None
    }

    fn request_layout(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        window: &mut Window,
        cx: &mut App,
    ) -> (LayoutId, ()) {
        // `with_rem_size` is only valid during prepaint/paint; layout sizing here
        // (flex fill, padding) intentionally stays at the base rem.
        (self.child.request_layout(window, cx), ())
    }

    fn prepaint(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        _bounds: Bounds<Pixels>,
        _request_layout: &mut Self::RequestLayoutState,
        window: &mut Window,
        cx: &mut App,
    ) {
        let rem = self.rem;
        window.with_rem_size(Some(rem), |window| {
            self.child.prepaint(window, cx);
        });
    }

    fn paint(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        _bounds: Bounds<Pixels>,
        _request_layout: &mut Self::RequestLayoutState,
        _prepaint: &mut Self::PrepaintState,
        window: &mut Window,
        cx: &mut App,
    ) {
        let rem = self.rem;
        window.with_rem_size(Some(rem), |window| {
            self.child.paint(window, cx);
        });
    }
}
