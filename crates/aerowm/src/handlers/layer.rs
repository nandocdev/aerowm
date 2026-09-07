use smithay::delegate_layer_shell;
use smithay::desktop::{LayerSurface as DesktopLayerSurface, layer_map_for_output};
use smithay::output::Output;
use smithay::reexports::wayland_server::protocol::wl_output::WlOutput;
use smithay::wayland::shell::wlr_layer::{
    Layer, LayerSurface as WlrLayerSurface, WlrLayerShellHandler, WlrLayerShellState,
};

use crate::state::AerowmState;

impl WlrLayerShellHandler for AerowmState {
    fn shell_state(&mut self) -> &mut WlrLayerShellState {
        &mut self.layer_shell_state
    }

    fn new_layer_surface(
        &mut self,
        surface: WlrLayerSurface,
        output: Option<WlOutput>,
        layer: Layer,
        namespace: String,
    ) {
        // Resolve the target output: the one the client asked for, else primary.
        let out = output
            .as_ref()
            .and_then(Output::from_resource)
            .or_else(|| self.output.clone())
            .or_else(|| self.space.outputs().next().cloned());

        let Some(out) = out else {
            tracing::warn!("layer surface with no output available, closing");
            surface.send_close();
            return;
        };

        let desktop_layer = DesktopLayerSurface::new(surface.clone(), namespace.clone());
        tracing::info!("new layer surface '{namespace}' on {}", out.name());

        {
            let mut map = layer_map_for_output(&out);
            if let Err(e) = map.map_layer(&desktop_layer) {
                tracing::warn!("failed to map layer surface: {e:?}");
                surface.send_close();
                return;
            }
        }

        // Suggest a size: full output span on the anchored axis so bars
        // (top/bottom/left/right) get a sane initial configure. The client
        // still decides its final size via anchor + exclusive zone.
        let output_size = out
            .current_mode()
            .map(|m| m.size)
            .unwrap_or((1920, 1080).into());
        surface.with_pending_state(|state| {
            // Advisory size; the client decides via anchor + exclusive zone.
            // Top layers (bars) conventionally span the full width and pick
            // their own height, so suggest that. Full-screen layers
            // (backgrounds, lock screens) get the whole output.
            state.size = Some(match layer {
                Layer::Top | Layer::Bottom => (output_size.w, 0).into(),
                Layer::Background | Layer::Overlay => (output_size.w, output_size.h).into(),
            });
        });
        surface.send_configure();

        // Exclusive zones changed → re-tile windows around the new layer.
        self.apply_layout();
        self.update_keyboard_focus();
    }

    fn layer_destroyed(&mut self, surface: WlrLayerSurface) {
        tracing::info!("layer surface destroyed");
        // Find and unmap from whichever output holds it.
        let outputs: Vec<_> = self.space.outputs().cloned().collect();
        for out in &outputs {
            let target = {
                let map = layer_map_for_output(out);
                map.layers()
                    .find(|l| l.layer_surface() == &surface)
                    .cloned()
            };
            if let Some(layer) = target {
                layer_map_for_output(out).unmap_layer(&layer);
            }
        }
        self.apply_layout();
        self.update_keyboard_focus();
        drop(surface);
    }
}

delegate_layer_shell!(AerowmState);
