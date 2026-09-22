#[no_mangle]
pub extern "C" fn on_request_headers(context_id: u32) -> i32 {
    // If context_id is 99, deny access (2 = AccessDenied)
    // Otherwise allow (0 = Continue)
    if context_id == 99 {
        2 // AccessDenied
    } else if context_id == 1 {
        1 // HeaderRewrite
    } else {
        0 // Continue
    }
}
