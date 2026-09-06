use mlua::{Function, Lua, Result, Table};
use std::path::Path;

/// The central Lua(u) scripting engine for AeroWM.
/// It wraps the Luau VM, injects the secure API bindings, 
/// and handles the evaluation of user configurations.
pub struct ScriptEngine {
    lua: Lua,
}

impl ScriptEngine {
    /// Initializes a new Luau sandbox environment with the `aerowm` API.
    pub fn new() -> Result<Self> {
        // Creates a new Luau state (guaranteed by the `luau` feature in Cargo.toml)
        let lua = Lua::new();
        
        // Expose a global table for the AeroWM API
        let aerowm_table = lua.create_table()?;
        
        // Basic logging binding from Luau to Rust stdout
        let log_fn = lua.create_function(|_, msg: String| {
            println!("[AeroWM-Luau] {}", msg);
            Ok(())
        })?;
        aerowm_table.set("log", log_fn)?;
        
        // Table dedicated to user-defined lifecycle hooks
        let hooks_table = lua.create_table()?;
        aerowm_table.set("hooks", hooks_table)?;

        // Inject the `aerowm` table into the Luau globals
        lua.globals().set("aerowm", aerowm_table)?;

        Ok(Self { lua })
    }

    /// Safely evaluates a configuration file from disk.
    /// Any syntax or runtime error is caught and returned as an `mlua::Result`,
    /// preventing the main compositor process from panicking.
    pub fn load_config_file<P: AsRef<Path>>(&self, path: P) -> Result<()> {
        let source = std::fs::read_to_string(path)
            .map_err(|e| mlua::Error::RuntimeError(format!("Failed to read config file: {}", e)))?;
        
        self.load_config_string(&source)
    }

    /// Evaluates raw Luau code string. Useful for hot-reloads and testing.
    pub fn load_config_string(&self, code: &str) -> Result<()> {
        self.lua.load(code).exec()?;
        Ok(())
    }

    /// Emits a lifecycle event (hook) to the Luau environment.
    /// E.g. "window_opened", "focus_changed".
    /// If the user script defined a function for this hook, it is executed safely.
    pub fn emit_hook(&self, hook_name: &str) -> Result<()> {
        let globals = self.lua.globals();
        let aerowm: Table = globals.get("aerowm")?;
        let hooks: Table = aerowm.get("hooks")?;
        
        // Only call the hook if the user defined it as a function
        if let Ok(hook_fn) = hooks.get::<Function>(hook_name) {
            hook_fn.call::<()>(())?;
        }
        
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_engine_initialization() {
        let engine = ScriptEngine::new().expect("Failed to init engine");
        assert!(engine.load_config_string("aerowm.log('Hello from Luau')").is_ok());
    }

    #[test]
    fn test_hooks_registration() {
        let engine = ScriptEngine::new().unwrap();
        
        let config = r#"
            aerowm.hooks.window_opened = function()
                aerowm.log("Hook: window_opened executed successfully!")
            end
        "#;
        
        engine.load_config_string(config).unwrap();
        // Emitting the hook should succeed
        assert!(engine.emit_hook("window_opened").is_ok());
        
        // Emitting an unregistered hook should also succeed silently (do nothing)
        assert!(engine.emit_hook("unregistered_hook").is_ok());
    }

    #[test]
    fn test_syntax_error_isolation() {
        let engine = ScriptEngine::new().unwrap();
        // Malformed Luau syntax
        let bad_config = "aerowm.log('Missing closing parenthesis'";
        
        let result = engine.load_config_string(bad_config);
        // Ensure the error is caught and doesn't panic
        assert!(result.is_err());
    }
}
