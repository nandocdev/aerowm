use aerowm_lua::ScriptEngine;
use wayland_server::DisplayHandle;

pub struct AerowmState {
    pub engine: ScriptEngine,
    pub is_running: bool,
    // Note: Smithay compositor states (CompositorState, ShmState, XdgShellState, etc.)
    // will be added here. They require implementing extensive trait delegates 
    // (`delegate_compositor!`, `delegate_shm!`, etc.) which we will build iteratively.
}

impl AerowmState {
    pub fn new(_display_handle: &DisplayHandle, engine: ScriptEngine) -> Self {
        Self {
            engine,
            is_running: true,
        }
    }
}
