//! Native hardware backend (`udev` / DRM-KMS) for Sprint 5.
//!
//! * Opens DRM devices through a logind session (`libseat`).
//! * Allocates scanout buffers with GBM and renders with GLES.
//! * Detects monitors dynamically (connect / disconnect) and advertises
//!   their native mode + refresh rate as Wayland outputs (V-Sync is driven
//!   by DRM VBlank events).
//! * Presents through `GbmBufferedSurface`, whose `DrmSurface` commits
//!   atomically whenever the driver supports it (`is_legacy() == false`).
//! * Feeds physical keyboards / mice through libinput into the shared
//!   `crate::input` dispatch (same keybindings as the nested backend).

use std::collections::{HashMap, HashSet};
use std::path::Path;

use smithay::backend::allocator::{
    Fourcc,
    gbm::{GbmAllocator, GbmBufferFlags, GbmDevice},
};
use smithay::backend::drm::{DrmDevice, DrmDeviceFd, DrmEvent, DrmEventMetadata, DrmNode, GbmBufferedSurface};
use smithay::backend::egl::{EGLContext, EGLDisplay};
use smithay::backend::libinput::{LibinputInputBackend, LibinputSessionInterface};
use smithay::backend::renderer::{
    Bind, ImportEgl, damage::OutputDamageTracker, gles::GlesRenderer,
};
use smithay::backend::session::{Session, libseat::LibSeatSession};
use smithay::backend::udev::{UdevBackend, UdevEvent, all_gpus, primary_gpu};
use smithay::output::{Mode as OutputMode, Output, PhysicalProperties, Scale};
use smithay::reexports::calloop::{EventLoop, LoopHandle};
use smithay::reexports::drm::control::{
    connector, crtc, Mode, ModeTypeFlags, Device as ControlDevice,
};
use smithay::reexports::input::Libinput;
use smithay::reexports::rustix::fs::OFlags;
use smithay::utils::{DeviceFd, Point, Size};
use tracing::{debug, error, info, warn};
use wayland_server::{Display, DisplayHandle};

use crate::state::AerowmState;

/// Preferred scanout formats, 10-bit first like anvil.
const COLOR_FORMATS: &[Fourcc] = &[
    Fourcc::Abgr2101010,
    Fourcc::Argb2101010,
    Fourcc::Abgr8888,
    Fourcc::Argb8888,
];

const CLEAR_COLOR: [f32; 4] = [0.08, 0.08, 0.1, 1.0];

type GbmSurface = GbmBufferedSurface<GbmAllocator<DrmDeviceFd>, ()>;

/// Render state for a single connected monitor.
pub struct UdevSurface {
    pub output: Output,
    pub connector: connector::Handle,
    // Kept for future dynamic modeset (`use_mode`) and VRR work.
    #[allow(dead_code)]
    pub crtc: crtc::Handle,
    #[allow(dead_code)]
    pub drm_mode: Mode,
    pub gbm_surface: GbmSurface,
    pub damage_tracker: OutputDamageTracker,
    pub renderer: GlesRenderer,
    rendering: bool,
}

/// One DRM device (GPU) with all its active connectors.
pub struct UdevDevice {
    // Duplicates the map key; kept for logging without re-borrowing.
    #[allow(dead_code)]
    pub node: DrmNode,
    pub drm: DrmDevice,
    pub gbm: GbmDevice<DrmDeviceFd>,
    pub surfaces: HashMap<crtc::Handle, UdevSurface>,
}

/// Runtime storage for the native backend, kept inside `AerowmState`.
pub struct UdevRuntime {
    pub session: LibSeatSession,
    pub display_handle: DisplayHandle,
    pub loop_handle: LoopHandle<'static, AerowmState>,
    pub devices: HashMap<DrmNode, UdevDevice>,
    // Owns the context; suspend/resume uses a clone registered at init.
    #[allow(dead_code)]
    pub libinput: Libinput,
}

/// Output ↔ (device, crtc) association stored in the `Space`.
#[derive(Debug, PartialEq, Clone, Copy)]
struct UdevOutputId {
    device: DrmNode,
    crtc: crtc::Handle,
}

/// Initializes the native backend: session, GPU enumeration, udev hotplug,
/// libinput and per-connector outputs. Returns `Err` when no usable DRM
/// device exists (the caller may then fall back to the nested backend).
pub fn init_udev(
    event_loop: &mut EventLoop<'static, AerowmState>,
    display: &mut Display<AerowmState>,
    state: &mut AerowmState,
) -> Result<(), Box<dyn std::error::Error>> {
    info!("Initializing Udev Backend (native DRM/KMS)");

    let display_handle = display.handle();
    let loop_handle: LoopHandle<'static, AerowmState> = event_loop.handle();

    // --- logind session ------------------------------------------------------
    let (session, session_notifier) =
        LibSeatSession::new().map_err(|e| format!("libseat session: {e}"))?;
    let seat_name = session.seat();
    info!("libseat session on seat {seat_name}");

    // --- libinput ------------------------------------------------------------
    let mut libinput =
        Libinput::new_with_udev::<LibinputSessionInterface<LibSeatSession>>(session.clone().into());
    libinput
        .udev_assign_seat(&seat_name)
        .map_err(|e| format!("libinput assign seat: {e:?}"))?;
    let libinput_backend = LibinputInputBackend::new(libinput.clone());

    state.udev_data = Some(UdevRuntime {
        session,
        display_handle: display_handle.clone(),
        loop_handle: loop_handle.clone(),
        devices: HashMap::new(),
        libinput: libinput.clone(),
    });

    // --- GPU inventory (logging) ----------------------------------------------
    match primary_gpu(&seat_name) {
        Ok(Some(path)) => info!("primary GPU: {}", path.display()),
        Ok(None) => info!("no primary GPU reported, using first available"),
        Err(e) => warn!("primary_gpu query failed: {e:?}"),
    }
    match all_gpus(&seat_name) {
        Ok(gpus) => info!("GPUs on seat {seat_name}: {gpus:?}"),
        Err(e) => warn!("all_gpus query failed: {e:?}"),
    }

    // --- session pause / resume ------------------------------------------------
    {
        use smithay::backend::session::Event as SessionEvent;
        let mut libinput_suspend = libinput.clone();
        loop_handle
            .insert_source(session_notifier, move |event, &mut (), state: &mut AerowmState| {
                match event {
                    SessionEvent::PauseSession => {
                        info!("session paused");
                        libinput_suspend.suspend();
                        if let Some(runtime) = state.udev_data.as_mut() {
                            for device in runtime.devices.values_mut() {
                                device.drm.pause();
                            }
                        }
                    }
                    SessionEvent::ActivateSession => {
                        info!("session activated");
                        if let Err(e) = libinput_suspend.resume() {
                            error!("libinput resume failed: {e:?}");
                        }
                        let nodes: Vec<DrmNode> = state
                            .udev_data
                            .as_ref()
                            .map(|r| r.devices.keys().copied().collect())
                            .unwrap_or_default();
                        if let Some(runtime) = state.udev_data.as_mut() {
                            for node in &nodes {
                                if let Some(device) = runtime.devices.get_mut(node)
                                    && let Err(e) = device.drm.activate(false)
                                {
                                    error!("DRM activate failed for {node:?}: {e:?}");
                                }
                            }
                        }
                        for node in nodes {
                            render_device(state, node);
                        }
                    }
                }
            })
            .map_err(|e| format!("session event source: {e}"))?;
    }

    // --- udev hotplug ------------------------------------------------------------
    let udev_backend = UdevBackend::new(&seat_name).map_err(|e| format!("udev backend: {e}"))?;

    // Bring up already-present GPUs first.
    for (device_id, path) in udev_backend.device_list() {
        match DrmNode::from_dev_id(device_id) {
            Ok(node) => {
                if let Err(e) = device_added(state, &display_handle, &loop_handle, node, &path) {
                    warn!("skipping DRM device {node:?} ({}): {e}", path.display());
                }
            }
            Err(e) => warn!("bad DRM node for device {device_id}: {e:?}"),
        }
    }

    if state
        .udev_data
        .as_ref()
        .map(|r| r.devices.is_empty())
        .unwrap_or(true)
    {
        return Err("no usable DRM devices found".into());
    }

    loop_handle
        .insert_source(udev_backend, move |event, _, state: &mut AerowmState| {
            let (dh, lh) = match state.udev_data.as_ref() {
                Some(r) => (r.display_handle.clone(), r.loop_handle.clone()),
                None => return,
            };
            match event {
                UdevEvent::Added { device_id, path } => match DrmNode::from_dev_id(device_id) {
                    Ok(node) => {
                        if let Err(e) = device_added(state, &dh, &lh, node, &path) {
                            warn!("hotplug add failed for {node:?}: {e}");
                        }
                    }
                    Err(e) => warn!("bad hotplug node {device_id}: {e:?}"),
                },
                UdevEvent::Changed { device_id } => {
                    if let Ok(node) = DrmNode::from_dev_id(device_id) {
                        device_changed(state, &dh, node);
                    }
                }
                UdevEvent::Removed { device_id } => {
                    if let Ok(node) = DrmNode::from_dev_id(device_id) {
                        device_removed(state, node);
                    }
                }
            }
        })
        .map_err(|e| format!("udev event source: {e}"))?;

    // --- libinput events → shared input dispatch ----------------------------------
    loop_handle
        .insert_source(libinput_backend, move |event, _, state: &mut AerowmState| {
            crate::input::handle_libinput_event(state, event);
        })
        .map_err(|e| format!("libinput event source: {e}"))?;

    info!("Udev backend initialized");
    Ok(())
}

// ---------------------------------------------------------------------------
// Device / connector management
// ---------------------------------------------------------------------------

fn open_drm_fd(state: &mut AerowmState, path: &Path) -> Result<DrmDeviceFd, Box<dyn std::error::Error>> {
    let runtime = state
        .udev_data
        .as_mut()
        .ok_or("udev runtime not initialized")?;
    let fd = runtime
        .session
        .open(
            path,
            OFlags::RDWR | OFlags::CLOEXEC | OFlags::NOCTTY | OFlags::NONBLOCK,
        )
        .map_err(|e| format!("open {}: {e}", path.display()))?;
    Ok(DrmDeviceFd::new(DeviceFd::from(fd)))
}

fn device_added(
    state: &mut AerowmState,
    display_handle: &DisplayHandle,
    loop_handle: &LoopHandle<'static, AerowmState>,
    node: DrmNode,
    path: &Path,
) -> Result<(), Box<dyn std::error::Error>> {
    if state
        .udev_data
        .as_ref()
        .map(|r| r.devices.contains_key(&node))
        .unwrap_or(false)
    {
        return Ok(());
    }
    info!("adding DRM device {node:?} at {}", path.display());

    let fd = open_drm_fd(state, path)?;
    let (drm, notifier) =
        DrmDevice::new(fd.clone(), true).map_err(|e| format!("DrmDevice: {e:?}"))?;
    let gbm = GbmDevice::new(fd).map_err(|e| format!("GbmDevice: {e}"))?;

    // VBlank → frame_submitted + render next frame (this is the V-Sync path).
    let vblank_node = node;
    loop_handle
        .insert_source(
            notifier,
            move |event, metadata: &mut Option<DrmEventMetadata>, state: &mut AerowmState| {
                if let DrmEvent::VBlank(crtc) = event {
                    let _ = metadata;
                    on_vblank(state, vblank_node, crtc);
                }
            },
        )
        .map_err(|e| format!("drm notifier source: {e}"))?;

    let runtime = state.udev_data.as_mut().ok_or("udev runtime gone")?;
    runtime.devices.insert(
        node,
        UdevDevice {
            node,
            drm,
            gbm,
            surfaces: HashMap::new(),
        },
    );

    // Scan connectors and light up everything connected.
    device_changed(state, display_handle, node);
    Ok(())
}

/// (Re-)scans connectors of a device: lights up newly connected monitors
/// with their native mode and tears down disconnected ones.
fn device_changed(state: &mut AerowmState, display_handle: &DisplayHandle, node: DrmNode) {
    // Snapshot connector states without holding borrows.
    let snapshot: Vec<(connector::Handle, connector::State, Vec<Mode>)> = {
        let Some(runtime) = state.udev_data.as_ref() else {
            return;
        };
        let Some(device) = runtime.devices.get(&node) else {
            return;
        };
        let Ok(res) = device.drm.resource_handles() else {
            warn!("resource_handles failed for {node:?}");
            return;
        };
        res.connectors()
            .iter()
            .filter_map(|h| {
                device
                    .drm
                    .get_connector(*h, false)
                    .ok()
                    .map(|info| (*h, info.state(), info.modes().to_vec()))
            })
            .collect()
    };

    let mut used_crtcs: HashSet<crtc::Handle> = state
        .udev_data
        .as_ref()
        .and_then(|r| r.devices.get(&node))
        .map(|d| d.surfaces.keys().copied().collect())
        .unwrap_or_default();

    for (conn_handle, conn_state, modes) in snapshot {
        let connected = conn_state == connector::State::Connected && !modes.is_empty();
        let has_surface = state
            .udev_data
            .as_ref()
            .and_then(|r| r.devices.get(&node))
            .map(|d| d.surfaces.values().any(|s| s.connector == conn_handle))
            .unwrap_or(false);

        if connected && !has_surface {
            match connector_connected(state, display_handle, node, conn_handle, &modes) {
                Ok(crtc) => {
                    used_crtcs.insert(crtc);
                }
                Err(e) => warn!("connector setup failed on {node:?}: {e}"),
            }
        } else if !connected && has_surface {
            connector_disconnected(state, node, conn_handle);
        }
    }
}

fn pick_mode(modes: &[Mode]) -> Mode {
    modes
        .iter()
        .find(|m| m.mode_type().contains(ModeTypeFlags::PREFERRED))
        .or_else(|| {
            modes
                .iter()
                .max_by_key(|m| m.size().0 as u32 * m.size().1 as u32)
        })
        .copied()
        .expect("non-empty modes checked by caller")
}

/// Finds a free CRTC compatible with the connector (current encoder first).
fn find_crtc(
    drm: &DrmDevice,
    conn_handle: connector::Handle,
    used: &HashSet<crtc::Handle>,
) -> Option<crtc::Handle> {
    let conn = drm.get_connector(conn_handle, false).ok()?;
    let res = drm.resource_handles().ok()?;

    // Prefer the CRTC the connector is already using.
    for enc_handle in conn.encoders() {
        if let Ok(enc) = drm.get_encoder(*enc_handle)
            && let Some(crtc) = enc.crtc()
            && !used.contains(&crtc)
            && res.crtcs().contains(&crtc)
        {
            return Some(crtc);
        }
    }
    // Otherwise any compatible, unused CRTC.
    for enc_handle in conn.encoders() {
        let Ok(enc) = drm.get_encoder(*enc_handle) else {
            continue;
        };
        for crtc in res.filter_crtcs(enc.possible_crtcs()) {
            if !used.contains(&crtc) {
                return Some(crtc);
            }
        }
    }
    None
}

/// Lights up one connector: native mode, Wayland output, GBM surface.
fn connector_connected(
    state: &mut AerowmState,
    display_handle: &DisplayHandle,
    node: DrmNode,
    conn_handle: connector::Handle,
    modes: &[Mode],
) -> Result<crtc::Handle, Box<dyn std::error::Error>> {
    let drm_mode = pick_mode(modes);
    let (w, h) = drm_mode.size();
    let refresh = drm_mode.vrefresh();
    info!("connector on {node:?}: native mode {w}x{h}@{refresh}Hz");

    // --- CRTC -----------------------------------------------------------------
    let used_crtcs: HashSet<crtc::Handle> = state
        .udev_data
        .as_ref()
        .and_then(|r| r.devices.get(&node))
        .map(|d| d.surfaces.keys().copied().collect())
        .unwrap_or_default();
    let crtc = {
        let runtime = state.udev_data.as_ref().ok_or("udev runtime gone")?;
        let device = runtime.devices.get(&node).ok_or("device gone")?;
        find_crtc(&device.drm, conn_handle, &used_crtcs).ok_or("no free CRTC")?
    };

    // --- Wayland output with native mode + refresh (V-Sync source) -------------
    let (conn_kind, conn_id, phys_mm, subpixel) = {
        let runtime = state.udev_data.as_ref().ok_or("udev runtime gone")?;
        let device = runtime.devices.get(&node).ok_or("device gone")?;
        let info = device
            .drm
            .get_connector(conn_handle, false)
            .map_err(|e| format!("connector: {e:?}"))?;
        (
            format!("{:?}", info.interface()),
            info.interface_id(),
            info.size().unwrap_or((0, 0)),
            info.subpixel(),
        )
    };
    let output_name = format!("{conn_kind}-{conn_id}");
    let output = Output::new(
        output_name.clone(),
        PhysicalProperties {
            size: Size::from((phys_mm.0 as i32, phys_mm.1 as i32)),
            subpixel: subpixel.into(),
            make: "AeroWM".into(),
            model: "DRM".into(),
        },
    );
    let wl_mode = OutputMode {
        size: Size::from((w as i32, h as i32)),
        refresh: refresh as i32,
    };
    output.add_mode(wl_mode);
    output.set_preferred(wl_mode);
    output.create_global::<AerowmState>(display_handle);
    output
        .user_data()
        .insert_if_missing(|| UdevOutputId { device: node, crtc });

    // Side-by-side placement.
    let x = state
        .space
        .outputs()
        .fold(0, |acc, o| {
            acc + state.space.output_geometry(o).map(|g| g.size.w).unwrap_or(0)
        });
    let position = Point::from((x, 0));
    output.change_current_state(Some(wl_mode), None, Some(Scale::Integer(1)), Some(position));
    state.space.map_output(&output, position);
    state
        .output_size
        .get_or_insert(Size::from((w as i32, h as i32)));
    // Tile the active workspace on the usable monitor area (exclusive
    // zones of layer-shell bars/panels are deducted automatically).
    let area = state.usable_area_for_output(&output);
    state.apply_layout_in(area);

    // --- DRM surface + GBM swapchain -------------------------------------------
    // GLES renderer on top of the GBM device.
    let (render_formats, renderer) = {
        let runtime = state.udev_data.as_ref().ok_or("udev runtime gone")?;
        let device = runtime.devices.get(&node).ok_or("device gone")?;
        let egl_display = unsafe { EGLDisplay::new(device.gbm.clone()) }
            .map_err(|e| format!("EGLDisplay: {e:?}"))?;
        let egl_context =
            EGLContext::new(&egl_display).map_err(|e| format!("EGLContext: {e:?}"))?;
        let mut renderer =
            unsafe { GlesRenderer::new(egl_context) }.map_err(|e| format!("GlesRenderer: {e:?}"))?;
        if let Err(e) = renderer.bind_wl_display(display_handle) {
            debug!("EGL display binding failed (non-fatal): {e:?}");
        }
        let formats = renderer
            .egl_context()
            .dmabuf_render_formats()
            .iter()
            .copied()
            .collect::<Vec<_>>();
        (formats, renderer)
    };

    let runtime = state.udev_data.as_mut().ok_or("udev runtime gone")?;
    let device = runtime.devices.get_mut(&node).ok_or("device gone")?;

    let drm_surface = device
        .drm
        .create_surface(crtc, drm_mode, &[conn_handle])
        .map_err(|e| format!("create_surface: {e:?}"))?;
    // Atomic commit path when the driver supports it, legacy page-flip otherwise.
    info!(
        "connector {output_name}: {} modesetting",
        if drm_surface.is_legacy() {
            "legacy page-flip"
        } else {
            "atomic"
        }
    );

    let allocator = GbmAllocator::new(
        device.gbm.clone(),
        GbmBufferFlags::RENDERING | GbmBufferFlags::SCANOUT,
    );

    let gbm_surface = GbmBufferedSurface::new(drm_surface, allocator, COLOR_FORMATS, render_formats)
        .map_err(|e| format!("GbmBufferedSurface: {e:?}"))?;

    device.surfaces.insert(
        crtc,
        UdevSurface {
            output: output.clone(),
            connector: conn_handle,
            crtc,
            drm_mode,
            gbm_surface,
            damage_tracker: OutputDamageTracker::from_output(&output),
            renderer,
            rendering: false,
        },
    );

    // First frame immediately so the monitor lights up without waiting.
    render_udev_surface(state, node, crtc);
    Ok(crtc)
}

fn connector_disconnected(state: &mut AerowmState, node: DrmNode, conn_handle: connector::Handle) {
    let crtc = state
        .udev_data
        .as_ref()
        .and_then(|r| r.devices.get(&node))
        .and_then(|d| {
            d.surfaces
                .iter()
                .find(|(_, s)| s.connector == conn_handle)
                .map(|(c, _)| *c)
        });
    let Some(crtc) = crtc else { return };
    info!("connector disconnected on {node:?}, tearing down crtc {crtc:?}");

    if let Some(runtime) = state.udev_data.as_mut()
        && let Some(device) = runtime.devices.get_mut(&node)
    {
        device.surfaces.remove(&crtc);
    }
    let output = state
        .space
        .outputs()
        .find(|o| {
            o.user_data()
                .get::<UdevOutputId>()
                .map(|id| id.device == node && id.crtc == crtc)
                .unwrap_or(false)
        })
        .cloned();
    if let Some(output) = output {
        state.space.unmap_output(&output);
    }
    state.space.refresh();
}

fn device_removed(state: &mut AerowmState, node: DrmNode) {
    info!("removing DRM device {node:?}");
    let conns: Vec<connector::Handle> = state
        .udev_data
        .as_ref()
        .and_then(|r| r.devices.get(&node))
        .map(|d| d.surfaces.values().map(|s| s.connector).collect())
        .unwrap_or_default();
    for conn in conns {
        connector_disconnected(state, node, conn);
    }
    if let Some(runtime) = state.udev_data.as_mut() {
        runtime.devices.remove(&node);
    }
}

// ---------------------------------------------------------------------------
// Rendering (V-Sync driven)
// ---------------------------------------------------------------------------

/// Renders one frame on the given connector and queues it for scanout.
/// The underlying `DrmSurface` commits atomically when supported.
pub fn render_udev_surface(state: &mut AerowmState, node: DrmNode, crtc: crtc::Handle) {
    // Borrow split: the device map and the space are disjoint fields.
    let output = {
        let Some(runtime) = state.udev_data.as_mut() else {
            return;
        };
        let Some(device) = runtime.devices.get_mut(&node) else {
            return;
        };
        let Some(surface) = device.surfaces.get_mut(&crtc) else {
            return;
        };
        if surface.rendering {
            return;
        }
        surface.rendering = true;

        let output = surface.output.clone();
        let renderer = &mut surface.renderer;

        let mut lock_elements = Vec::new();
        if state.is_locked {
            if let Some(lock_surface) = state.lock_surfaces.first() {
                use smithay::backend::renderer::element::surface::{render_elements_from_surface_tree, WaylandSurfaceRenderElement};
                use smithay::backend::renderer::element::Kind;
                let mut tree = render_elements_from_surface_tree::<_, WaylandSurfaceRenderElement<GlesRenderer>>(
                    renderer,
                    lock_surface.wl_surface(),
                    (0, 0),
                    1.0,
                    1.0,
                    Kind::Unspecified,
                );
                lock_elements.append(&mut tree);
            }
        }

        let elements = if !state.is_locked {
            match state.space.render_elements_for_output(renderer, &output, 1.0) {
                Ok(e) => e,
                Err(e) => {
                    warn!("render elements failed: {e:?}");
                    surface.rendering = false;
                    return;
                }
            }
        } else {
            Vec::new()
        };

        let (dmabuf, age) = match surface.gbm_surface.next_buffer() {
            Ok(v) => v,
            Err(e) => {
                warn!("next_buffer failed: {e:?}");
                surface.rendering = false;
                return;
            }
        };
        let mut dmabuf = dmabuf;
        let mut fb = match renderer.bind(&mut dmabuf) {
            Ok(fb) => fb,
            Err(e) => {
                warn!("renderer bind failed: {e:?}");
                surface.rendering = false;
                return;
            }
        };
        let res = match surface.damage_tracker.render_output(
            renderer,
            &mut fb,
            age as usize,
            &elements,
            CLEAR_COLOR,
        ) {
            Ok(res) => res,
            Err(e) => {
                warn!("render_output failed: {e:?}");
                surface.rendering = false;
                return;
            }
        };
        let damage = res.damage.cloned();
        let sync = res.sync;
        drop(fb);

        if let Err(e) = surface.gbm_surface.queue_buffer(Some(sync), damage, ()) {
            warn!("queue_buffer failed: {e:?}");
            surface.rendering = false;
            return;
        }
        surface.rendering = false;
        output
    };

    // Frame callbacks live outside the runtime borrow.
    let now = std::time::Duration::from_millis(0);
    state.space.elements().for_each(|window| {
        window.send_frame(&output, now, None, |_, _| Some(output.clone()));
    });
    state.space.refresh();
    output.cleanup();
}

/// Renders all connectors of a device (used after session resume).
pub fn render_device(state: &mut AerowmState, node: DrmNode) {
    let crtcs: Vec<crtc::Handle> = state
        .udev_data
        .as_ref()
        .and_then(|r| r.devices.get(&node))
        .map(|d| d.surfaces.keys().copied().collect())
        .unwrap_or_default();
    for crtc in crtcs {
        render_udev_surface(state, node, crtc);
    }
}

/// VBlank handler: recycle the submitted buffer, then render the next frame.
/// This closes the V-Sync loop at the monitor's native refresh rate.
fn on_vblank(state: &mut AerowmState, node: DrmNode, crtc: crtc::Handle) {
    let submitted = state
        .udev_data
        .as_mut()
        .and_then(|r| r.devices.get_mut(&node))
        .and_then(|d| d.surfaces.get_mut(&crtc))
        .map(|s| s.gbm_surface.frame_submitted());
    match submitted {
        Some(Ok(_)) => {}
        Some(Err(e)) => warn!("frame_submitted failed: {e:?}"),
        None => return,
    }
    render_udev_surface(state, node, crtc);
}
