-- AeroWM Example Configuration (Luau)
-- Copy this file to ~/.config/aerowm/config.luau

aerowm.log("Loading AeroWM configuration...")

local mod = aerowm.mods.Mod4 -- Super / Windows key
local alt = aerowm.mods.Mod1 -- Alt key
local shift = aerowm.mods.Shift

-- =========================================================
-- Keybindings
-- =========================================================

-- Terminal
aerowm.binds[mod .. "+Return"] = function()
    aerowm.spawn("kitty")
end

-- App Launcher
aerowm.binds[mod .. "+d"] = function()
    aerowm.spawn("fuzzel")
end

-- Close active window
aerowm.binds[mod .. "+Shift+q"] = function()
    aerowm.spawn("aerowm-ctl kill")
end

-- Hot Reload Configuration
aerowm.binds[mod .. "+Shift+c"] = function()
    aerowm.spawn("aerowm-ctl reload")
end

-- Exit AeroWM
aerowm.binds[mod .. "+Shift+e"] = function()
    aerowm.spawn("aerowm-ctl exit")
end

-- Restart AeroWM (keeps clients alive)
aerowm.binds[mod .. "+Shift+r"] = function()
    aerowm.spawn("aerowm-ctl restart")
end

-- Workspace switching
for i = 1, 9 do
    -- Mod + [1-9] to switch workspace
    aerowm.binds[mod .. "+" .. tostring(i)] = function()
        aerowm.spawn("aerowm-ctl workspace switch " .. tostring(i))
    end
end

aerowm.binds[mod .. "+Right"] = function()
    aerowm.spawn("aerowm-ctl workspace next")
end

aerowm.binds[mod .. "+Left"] = function()
    aerowm.spawn("aerowm-ctl workspace prev")
end

-- =========================================================
-- Window Rules (Declarative)
-- =========================================================

-- Make pavucontrol always float
table.insert(aerowm.rules, {
    match = { class = "pavucontrol" },
    set = { floating = true }
})

-- Force Discord to workspace 9
table.insert(aerowm.rules, {
    match = { class = "discord" },
    set = { workspace = 9 }
})

-- Force Firefox to workspace 2
table.insert(aerowm.rules, {
    match = { class = "firefox" },
    set = { workspace = 2 }
})

-- =========================================================
-- Lifecycle Hooks
-- =========================================================

-- Called when the compositor first starts (not on hot-reloads)
aerowm.hooks.startup = function()
    aerowm.log("AeroWM started.")
    -- aerowm.spawn("swaybg -i ~/wallpapers/bg.png")
    aerowm.spawn("aerowm-bar")
end

-- Called when the config is hot-reloaded
aerowm.hooks.reload = function()
    aerowm.log("AeroWM config reloaded.")
end

aerowm.log("AeroWM configuration loaded successfully!")
