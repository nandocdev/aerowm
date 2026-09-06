-- Type definitions for AeroWM Luau Configuration
-- This file provides autocomplete and strict typing in compatible editors (like VSCode with Luau LSP)

export type Modifiers = {
    Mod1: string,
    Mod4: string,
    Shift: string,
    Control: string,
}

export type Hooks = {
    window_opened: (() -> ())?,
    window_closed: (() -> ())?,
    focus_changed: (() -> ())?,
    workspace_switched: (() -> ())?,
    output_connected: (() -> ())?,
    startup: (() -> ())?,
}

export type AeroWM = {
    --- Prints a message safely to the compositor standard output
    log: (message: string) -> (),
    
    --- Spawns an external asynchronous process using the shell
    spawn: (command: string) -> (),
    
    --- Available keyboard modifiers
    mods: Modifiers,
    
    --- Dictionary of global keybindings mapped to callback functions
    binds: { [string]: () -> () },
    
    --- Lifecycle events where callbacks can be registered
    hooks: Hooks,
}

--- The global API object injected by the Compositor
declare aerowm: AeroWM
