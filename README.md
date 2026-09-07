<div align="center">
  <h1>AeroWM</h1>
  <p><strong>A high-performance, scriptable Wayland dynamic tiling window manager.</strong></p>
  <p>
    <img src="https://img.shields.io/badge/Language-Rust-orange?style=flat-square&logo=rust" alt="Rust" />
    <img src="https://img.shields.io/badge/Scripting-Luau-blue?style=flat-square&logo=lua" alt="Luau" />
    <img src="https://img.shields.io/badge/Protocol-Wayland-green?style=flat-square&logo=linux" alt="Wayland" />
    <img src="https://img.shields.io/badge/License-MIT-brightgreen?style=flat-square" alt="License" />
  </p>
</div>

---

**AeroWM** is a minimalist, incredibly fast, and memory-efficient Wayland compositor built with Rust and [Smithay](https://github.com/Smithay/smithay). It brings the dynamic tiling paradigm (inspired by Qtile and Xmonad) to the modern Wayland ecosystem, powered by a lightning-fast **Luau** (Roblox's safe Lua dialect) scripting engine for configuration and automation.

## ✨ Features

- 🦀 **Rust & Smithay Powered:** Memory safe, highly concurrent, and backed by a robust Wayland library.
- 🚀 **Blazing Fast Scripting:** Configuration is written in **Luau**, offering high-performance JIT/AOT evaluation, syntax safety, and atomic hot-reloads ($< 15\text{ms}$).
- 🔄 **Seamless Hot-Restarting:** Survives crashes and configuration reloads by keeping Wayland clients alive via FD inheritance (no dropped applications).
- 🧩 **Dynamic Tiling Engine:** Mathematical and deterministic layouts (Columns, MonadTall, Max) separated cleanly from the protocol handlers.
- 📦 **Legacy Support:** Optional `XWayland` support (compile-time flag) for older X11 applications, ensuring 0 memory overhead for pure Wayland users.
- 🖼️ **Damage-based Rendering:** Highly optimized memory and GPU usage by repainting only the exact pixel regions that change.
- 🛠️ **Built-in IPC & Bar:** Comes with a native `wlr-layer-shell` ready top-bar (`aerowm-bar`) and a CLI utility (`aerowm-ctl`) for external control.

## 📦 Installation

### Arch Linux

AeroWM can be built and installed using the provided `PKGBUILD`.

```bash
git clone https://github.com/nandocdev/aerowm.git
cd aerowm
makepkg -si
```

### From Source (Cargo)

Ensure you have the required Wayland and rendering dependencies installed (`libwayland`, `libxkbcommon`, `pixman`, `libinput`, `seatd`).

```bash
cargo build --release --locked --all-targets --features xwayland
```

The binaries (`aerowm`, `aerowm-bar`, `aerowm-ctl`) will be available in `target/release/`.

## ⚙️ Configuration

AeroWM is configured via `~/.config/aerowm/config.luau`. 

If a configuration file doesn't exist, AeroWM will start with a blank state. You can find a fully documented example configuration in the [`examples/`](./examples/config.luau) directory.

### Example `config.luau`

```lua
local mod = aerowm.mods.Mod4 -- Super / Windows key

-- Visual Settings (Gaps)
aerowm.gaps = {
    inner = 5,
    outer = 10,
}

-- Spawn a terminal
aerowm.binds[mod .. "+Return"] = function()
    aerowm.spawn("kitty")
end

-- Toggle Scratchpad
aerowm.binds[mod .. "+Shift+Return"] = function()
    aerowm.spawn("aerowm-ctl scratchpad")
end

-- Declarative Window Rules
table.insert(aerowm.rules, {
    match = { class = "pavucontrol" },
    set = { floating = true }
})
table.insert(aerowm.rules, {
    match = { class = "keepassxc" },
    set = { scratchpad = true }
})

-- Hooks
aerowm.hooks.startup = function()
    aerowm.log("AeroWM started.")
    aerowm.spawn("aerowm-bar")
end
```

## 🎮 Usage

You can start AeroWM directly from a TTY (udev/KMS backend) or nested inside another Wayland/X11 session (winit backend) for development:

```bash
# Start standalone
aerowm

# Or use the provided desktop entry in your Display Manager (GDM, SDDM, etc.)
```

### IPC Commands (`aerowm-ctl`)

AeroWM provides an IPC CLI for interacting with the compositor remotely:

```bash
aerowm-ctl workspace next        # Go to next workspace
aerowm-ctl workspace switch 3    # Switch to workspace 3
aerowm-ctl kill                  # Kill active window
aerowm-ctl scratchpad            # Toggle scratchpad workspace
aerowm-ctl reload                # Hot-reload config.luau atomically
aerowm-ctl restart               # Hot-restart compositor in place
```

## 🏗️ Architecture

AeroWM strictly follows a pragmatic **Modular Monolith** architecture:
- **`aerowm-core`**: A pure, protocol-agnostic Rust library that computes geometry, tracking tree state and layouts.
- **`aerowm-lua`**: The Luau sandbox and API bridge.
- **`aerowm-ipc`**: Shared IPC payload schemas.
- **`aerowm`**: The actual Smithay compositor bridging `udev`, `winit`, `layer-shell`, and the `core`.
- **`aerowm-ctl` / `aerowm-bar`**: Independent binaries interfacing via Unix Sockets and Wayland protocols.

## 🤝 Contributing

Contributions are welcome! If you plan to introduce new architecture or complexity, please read our principle:
> _The simplest solution that works today, can be maintained tomorrow, and evolved later, always wins._

## 📝 License

This project is licensed under the MIT License - see the [LICENSE](LICENSE) file for details.
