//! ContentEditable click and drag event interception.

use std::cell::RefCell;
use std::rc::Rc;

/// Click event data for contentEditable elements.
#[derive(Debug, Clone)]
pub struct ContentEditableClickData {
    /// The node ID of the contentEditable root element.
    pub ce_node_id: usize,
    /// Mouse X position relative to viewport.
    pub x: f32,
    /// Mouse Y position relative to viewport.
    pub y: f32,
    /// Click count: 1=single, 2=double, 3=triple.
    pub click_count: u8,
    /// Whether Shift is held (for extend-selection).
    pub shift: bool,
    /// The `data-table-cell` attribute value from the closest ancestor with
    /// that attribute, if any. Format: `"{table_id}:{row}:{col}"`.
    pub closest_table_cell: Option<String>,
}

/// Drag event data for contentEditable elements.
#[derive(Debug, Clone)]
pub struct ContentEditableDragData {
    /// The node ID of the contentEditable root element.
    pub ce_node_id: usize,
    /// Mouse X position relative to viewport.
    pub x: f32,
    /// Mouse Y position relative to viewport.
    pub y: f32,
}

/// Type alias for the CE click interceptor callback.
/// Returns true if the event was handled (app.rs should skip default CE handling).
pub type CeClickInterceptor = Rc<dyn Fn(&ContentEditableClickData) -> bool>;

/// Type alias for the CE drag interceptor callback.
/// Returns true if the event was handled.
pub type CeDragInterceptor = Rc<dyn Fn(&ContentEditableDragData) -> bool>;

thread_local! {
    static CE_CLICK_INTERCEPTOR: RefCell<Option<CeClickInterceptor>> = RefCell::new(None);
    static CE_DRAG_INTERCEPTOR: RefCell<Option<CeDragInterceptor>> = RefCell::new(None);
}

/// Set the global contentEditable click interceptor.
/// Only one interceptor can be active at a time.
pub fn set_ce_click_interceptor<F>(cb: F)
where
    F: Fn(&ContentEditableClickData) -> bool + 'static,
{
    CE_CLICK_INTERCEPTOR.with(|i| {
        *i.borrow_mut() = Some(Rc::new(cb));
    });
}

/// Clear the global contentEditable click interceptor.
pub fn clear_ce_click_interceptor() {
    CE_CLICK_INTERCEPTOR.with(|i| {
        *i.borrow_mut() = None;
    });
}

/// Dispatch a contentEditable click event to the interceptor.
/// Returns true if the event was handled.
pub fn dispatch_ce_click(data: &ContentEditableClickData) -> bool {
    CE_CLICK_INTERCEPTOR.with(|i| {
        if let Some(ref interceptor) = *i.borrow() {
            interceptor(data)
        } else {
            false
        }
    })
}

/// Set the global contentEditable drag interceptor.
pub fn set_ce_drag_interceptor<F>(cb: F)
where
    F: Fn(&ContentEditableDragData) -> bool + 'static,
{
    CE_DRAG_INTERCEPTOR.with(|i| {
        *i.borrow_mut() = Some(Rc::new(cb));
    });
}

/// Clear the global contentEditable drag interceptor.
pub fn clear_ce_drag_interceptor() {
    CE_DRAG_INTERCEPTOR.with(|i| {
        *i.borrow_mut() = None;
    });
}

/// Dispatch a contentEditable drag event to the interceptor.
/// Returns true if the event was handled.
pub fn dispatch_ce_drag(data: &ContentEditableDragData) -> bool {
    CE_DRAG_INTERCEPTOR.with(|i| {
        if let Some(ref interceptor) = *i.borrow() {
            interceptor(data)
        } else {
            false
        }
    })
}
