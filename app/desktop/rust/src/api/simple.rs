/// One call of the app's interface on a workspace directory: the name of the call and
/// its arguments as JSON, the answer as JSON (see `xl_mcp::app_call`). The engine is
/// part of the program; nothing is served.
pub fn call(workspace: String, op: String, args: String) -> Result<String, String> {
    xl_mcp::app_call(std::path::Path::new(&workspace), &op, &args)
}

#[flutter_rust_bridge::frb(init)]
pub fn init_app() {
    flutter_rust_bridge::setup_default_user_utils();
}
