use mlua::{Function, Lua, Result, Table};
use std::path::{Path, PathBuf};
use std::process::Command;

#[derive(Debug, Default, Clone)]
pub struct WindowRules {
    pub floating: Option<bool>,
    pub workspace: Option<usize>,
}

/// The central Lua(u) scripting engine for AeroWM.
/// It wraps the Luau VM, injects the secure API bindings, 
/// and handles the evaluation of user configurations.
pub struct ScriptEngine {
    lua: Lua,
}

impl ScriptEngine {
    /// Initializes a new Luau sandbox environment with the `aerowm` API.
    pub fn new() -> Result<Self> {
        // Creates a new Luau state
        let lua = Lua::new();
        
        // Expose a global table for the AeroWM API
        let aerowm_table = lua.create_table()?;
        
        // Basic logging binding from Luau to Rust stdout
        let log_fn = lua.create_function(|_, msg: String| {
            println!("[AeroWM-Luau] {}", msg);
            Ok(())
        })?;
        aerowm_table.set("log", log_fn)?;

        // Command execution binding: aero.spawn("kitty")
        let spawn_fn = lua.create_function(|_, cmd: String| {
            Command::new("sh")
                .arg("-c")
                .arg(&cmd)
                .spawn()
                .map_err(|e| mlua::Error::RuntimeError(format!("Failed to spawn {}: {}", cmd, e)))?;
            Ok(())
        })?;
        aerowm_table.set("spawn", spawn_fn)?;

        // Key Modifiers table
        let mods_table = lua.create_table()?;
        mods_table.set("Mod1", "Alt")?;
        mods_table.set("Mod4", "Super")?;
        mods_table.set("Shift", "Shift")?;
        mods_table.set("Control", "Control")?;
        aerowm_table.set("mods", mods_table)?;

        // Table for user-defined keybindings
        let binds_table = lua.create_table()?;
        aerowm_table.set("binds", binds_table)?;
        
        // Table dedicated to user-defined lifecycle hooks
        let hooks_table = lua.create_table()?;
        aerowm_table.set("hooks", hooks_table)?;

        // Table dedicated to window rules
        let rules_table = lua.create_table()?;
        aerowm_table.set("rules", rules_table)?;

        // Inject the `aerowm` table into the Luau globals
        lua.globals().set("aerowm", aerowm_table)?;

        Ok(Self { lua })
    }

    /// Resolves the default configuration path (~/.config/aerowm/config.luau)
    pub fn default_config_path() -> Option<PathBuf> {
        dirs::config_dir().map(|mut p| {
            p.push("aerowm");
            p.push("config.luau");
            p
        })
    }

    /// Safely evaluates the default configuration file if it exists.
    pub fn load_default_config(&self) -> Result<()> {
        if let Some(path) = Self::default_config_path() {
            if path.exists() {
                return self.load_config_file(path);
            }
        }
        Ok(())
    }

    /// Safely evaluates a configuration file from disk.
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

    /// Number of entries in `aerowm.rules`. Useful to verify a config
    /// actually registered rules (e.g. `aerowm check`, integration tests).
    pub fn rule_count(&self) -> usize {
        let globals = self.lua.globals();
        let aerowm: Table = match globals.get("aerowm") {
            Ok(t) => t,
            Err(_) => return 0,
        };
        let rules: Table = match aerowm.get("rules") {
            Ok(t) => t,
            Err(_) => return 0,
        };
        rules.len().unwrap_or(0).max(0) as usize
    }

    /// Emits a lifecycle event (hook) to the Luau environment.
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

    /// Dispatches a global keybinding: looks up `aerowm.binds[combo]` and calls it.
    /// Returns `true` if a bind existed (even if its execution failed — the
    /// error is logged and swallowed so the compositor never crashes).
    /// Combo format is canonical: e.g. `"Super+Return"`, `"Super+Shift+q"`.
    pub fn trigger_bind(&self, combo: &str) -> bool {
        let globals = self.lua.globals();
        let aerowm: Table = match globals.get("aerowm") {
            Ok(t) => t,
            Err(_) => return false,
        };
        let binds: Table = match aerowm.get("binds") {
            Ok(t) => t,
            Err(_) => return false,
        };
        let func: Function = match binds.get(combo) {
            Ok(f) => f,
            Err(_) => return false,
        };
        if let Err(e) = func.call::<()>(()) {
            eprintln!("[AeroWM-Luau] keybind '{combo}' failed: {e}");
        }
        true
    }

    /// Evaluates window rules against a given app class/id and title.
    /// Iterates through `aerowm.rules` and merges matches.
    pub fn evaluate_rules(&self, app_id: &str, title: Option<&str>) -> WindowRules {
        let mut result = WindowRules::default();
        
        let globals = self.lua.globals();
        let aerowm: Table = match globals.get("aerowm") {
            Ok(t) => t,
            Err(_) => return result,
        };
        let rules: Table = match aerowm.get("rules") {
            Ok(t) => t,
            Err(_) => return result,
        };
        
        for pair in rules.pairs::<mlua::Integer, Table>() {
            let (_, rule) = match pair {
                Ok(p) => p,
                Err(_) => continue,
            };
            
            let match_tbl: Table = match rule.get("match") {
                Ok(t) => t,
                Err(_) => continue,
            };
            
            let rule_class: Option<String> = match_tbl.get("class").ok();
            let rule_title: Option<String> = match_tbl.get("title").ok();
            
            let class_match = rule_class.map_or(true, |c| c == app_id);
            let title_match = rule_title.map_or(true, |t| title.map_or(false, |title| title.contains(&t)));
            
            if class_match && title_match {
                if let Ok(set_tbl) = rule.get::<Table>("set") {
                    // Absent key must stay `None` (a missing `floating`
                    // is not `false`): use Option so Nil maps to None.
                    if let Ok(Some(floating)) = set_tbl.get::<Option<bool>>("floating") {
                        result.floating = Some(floating);
                    }
                    if let Ok(ws) = set_tbl.get::<usize>("workspace") {
                        result.workspace = Some(ws);
                    }
                }
            }
        }
        
        result
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
        assert!(engine.emit_hook("window_opened").is_ok());
        assert!(engine.emit_hook("unregistered_hook").is_ok());
    }

    #[test]
    fn test_bindings_and_spawn() {
        let engine = ScriptEngine::new().unwrap();
        let config = r#"
            aerowm.binds[aerowm.mods.Mod4 .. "+Return"] = function()
                aerowm.spawn("echo 'test'")
            end
        "#;
        assert!(engine.load_config_string(config).is_ok());
    }

    #[test]
    fn test_trigger_bind() {
        let engine = ScriptEngine::new().unwrap();
        let config = r#"
            aerowm.binds["Super+q"] = function()
                aerowm.log("kill!")
            end
        "#;
        engine.load_config_string(config).unwrap();
        assert!(engine.trigger_bind("Super+q"));
        assert!(!engine.trigger_bind("Super+x"));
    }

    #[test]
    fn test_syntax_error_isolation() {
        let engine = ScriptEngine::new().unwrap();
        let bad_config = "aerowm.log('Missing closing parenthesis'";
        let result = engine.load_config_string(bad_config);
        assert!(result.is_err());
    }

    #[test]
    fn test_window_rules() {
        let engine = ScriptEngine::new().unwrap();
        let config = r#"
            table.insert(aerowm.rules, {
                match = { class = "kitty" },
                set = { floating = true, workspace = 3 }
            })
            table.insert(aerowm.rules, {
                match = { class = "firefox", title = "YouTube" },
                set = { workspace = 4 }
            })
        "#;
        engine.load_config_string(config).unwrap();
        
        let r1 = engine.evaluate_rules("kitty", None);
        assert_eq!(r1.floating, Some(true));
        assert_eq!(r1.workspace, Some(3));
        
        let r2 = engine.evaluate_rules("firefox", Some("YouTube - Mozilla Firefox"));
        assert_eq!(r2.workspace, Some(4));
        assert_eq!(r2.floating, None);
    }
}
