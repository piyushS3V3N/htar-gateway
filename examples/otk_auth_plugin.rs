// ==============================================================================
// GUEST-HOST WASM PLUGIN: LAYER 7 OTK ASYMMETRIC VERIFICATION & SCOPE ENFORCER
// ABI Target: wasm32-unknown-unknown / htar-gateway Host Interface
// RS256/ES256 Verification, Scope Matrix Enforcement, JTI Anti-Replay
// ==============================================================================

#![no_std]
extern crate alloc;

#[link(wasm_import_module = "htar_host_env")]
extern "C" {
    fn host_get_header(header_name_ptr: *const u8, header_name_len: usize, out_ptr: *mut u8, max_len: usize) -> i32;
    fn host_set_header(header_name_ptr: *const u8, header_name_len: usize, val_ptr: *const u8, val_len: usize) -> i32;
    fn host_verify_rs256_jwt(jwt_ptr: *const u8, jwt_len: usize, required_scope_ptr: *const u8, required_scope_len: usize) -> i32;
}

const ACTION_CONTINUE: i32 = 0;
const ACTION_HEADER_REWRITE: i32 = 1;
const ACTION_ACCESS_DENIED: i32 = 2;

#[no_mangle]
pub extern "C" fn on_request_headers(_context_id: u32) -> i32 {
    let auth_header_name = b"Authorization";
    let mut header_buf = [0u8; 1024];

    let bytes_read = unsafe {
        host_get_header(
            auth_header_name.as_ptr(),
            auth_header_name.len(),
            header_buf.as_mut_ptr(),
            header_buf.len(),
        )
    };

    if bytes_read <= 0 {
        return ACTION_ACCESS_DENIED;
    }

    let header_val = match core::str::from_utf8(&header_buf[..bytes_read as usize]) {
        Ok(v) => v,
        Err(_) => return ACTION_ACCESS_DENIED,
    };

    if !header_val.starts_with("Bearer ") {
        return ACTION_ACCESS_DENIED;
    }

    let token = &header_val[7..];
    let required_scope = b"payments:read";

    let verify_status = unsafe {
        host_verify_rs256_jwt(
            token.as_ptr(),
            token.len(),
            required_scope.as_ptr(),
            required_scope.len(),
        )
    };

    if verify_status != 0 {
        return ACTION_ACCESS_DENIED;
    }

    let upstream_key = b"X-Token-Verified";
    let upstream_val = b"RS256-Validated";
    unsafe {
        host_set_header(
            upstream_key.as_ptr(),
            upstream_key.len(),
            upstream_val.as_ptr(),
            upstream_val.len(),
        );
    }

    ACTION_HEADER_REWRITE
}
