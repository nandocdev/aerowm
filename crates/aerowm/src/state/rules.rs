//! Declarative window rules from the Luau config.
//!
//! Evaluation (which rules match, and therefore what should happen) is
//! kept separate from application (mutating workspaces, scratchpad and
//! floating state), so the mapping from `aerowm.rules` to effects can be
//! tested without a compositor.

use aerowm_core::id::WindowId;
use aerowm_lua::WindowRules;
use smithay::wayland::shell::xdg::ToplevelSurface;

use super::AerowmState;

/// A single outcome a window rule asks for. Decided before any state is
/// touched, then applied one at a time.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RuleEffect {
    /// Join a workspace, numbered 1-based exactly as in the config.
    MoveToWorkspace(usize),
    /// Hide in the scratchpad.
    Scratchpad,
    /// Keep user-controlled geometry instead of tiling.
    Float,
}

/// Maps evaluated rules to the effects to apply, in application order.
///
/// `Some(false)` is not an effect: rules only ever *add* state, they
/// never un-float or un-scratchpad a window.
pub fn evaluate_rule_effects(rules: &WindowRules) -> Vec<RuleEffect> {
    let mut effects = Vec::new();
    if let Some(workspace) = rules.workspace {
        effects.push(RuleEffect::MoveToWorkspace(workspace));
    }
    if rules.scratchpad == Some(true) {
        effects.push(RuleEffect::Scratchpad);
    }
    if rules.floating == Some(true) {
        effects.push(RuleEffect::Float);
    }
    effects
}

impl AerowmState {
    /// Applies declarative window rules via Luau to a new window.
    pub fn apply_window_rules(&mut self, id: WindowId, app_id: &str, title: Option<&str>) {
        let rules = self.engine.evaluate_rules(app_id, title);
        for effect in evaluate_rule_effects(&rules) {
            self.apply_rule_effect(id, effect);
        }
    }

    /// Performs one rule effect. Idempotent: rules are re-evaluated on
    /// every app_id/title change, so repeating one must not duplicate
    /// workspace or scratchpad entries.
    pub(crate) fn apply_rule_effect(&mut self, id: WindowId, effect: RuleEffect) {
        match effect {
            RuleEffect::MoveToWorkspace(ws_idx) => {
                let target_ws = ws_idx.saturating_sub(1);
                if target_ws >= self.workspaces.len() {
                    tracing::warn!(
                        "window rule targets workspace {ws_idx}, only {} exist; ignoring",
                        self.workspaces.len()
                    );
                } else if target_ws != self.active_ws {
                    for ws in &mut self.workspaces {
                        ws.remove_window(id);
                    }
                    self.workspaces[target_ws].add_window(id);
                }
            }
            RuleEffect::Scratchpad => {
                if !self.scratchpad.contains(&id) {
                    self.scratchpad.push(id);
                    for ws in &mut self.workspaces {
                        ws.remove_window(id);
                    }
                    if let Some(window) = self.window_object(id) {
                        self.space.unmap_elem(&window);
                    }
                    self.float_window(id);
                }
            }
            RuleEffect::Float => {
                self.float_window(id);
            }
        }
    }

    /// Re-evaluates Luau window rules for a mapped toplevel whose identity
    /// changed (see `app_id_changed`/`title_changed`: at `new_toplevel`
    /// time app_id/title are still empty). Idempotent.
    pub fn reapply_rules_for_toplevel(&mut self, surface: &ToplevelSurface) {
        let Some((&id, _)) = self.surfaces.iter().find(|(_, s)| *s == surface) else {
            return;
        };
        let app = crate::session::app_id_of_toplevel(surface);
        let app_id = app.as_ref().map(|a| a.id.as_str()).unwrap_or("");
        let title = app.as_ref().and_then(|a| a.title.as_deref());
        self.apply_window_rules(id, app_id, title);
        self.apply_layout();
        self.update_keyboard_focus();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn no_rules_produce_no_effects() {
        assert!(evaluate_rule_effects(&WindowRules::default()).is_empty());
    }

    #[test]
    fn disabled_flags_produce_no_effects() {
        // `false` is explicit but inert: rules never undo a placement.
        let rules = WindowRules {
            floating: Some(false),
            scratchpad: Some(false),
            workspace: None,
        };
        assert!(evaluate_rule_effects(&rules).is_empty());
    }

    #[test]
    fn each_flag_maps_to_its_effect() {
        // `false` in the middle is inert but does not stop the others.
        let rules = WindowRules {
            floating: Some(true),
            scratchpad: Some(false),
            workspace: Some(3),
        };
        assert_eq!(
            evaluate_rule_effects(&rules),
            vec![RuleEffect::MoveToWorkspace(3), RuleEffect::Float]
        );

        let rules = WindowRules {
            floating: Some(false),
            scratchpad: Some(true),
            workspace: None,
        };
        assert_eq!(evaluate_rule_effects(&rules), vec![RuleEffect::Scratchpad]);
    }

    #[test]
    fn effects_keep_application_order() {
        // Order matters: the workspace move runs first, then the window is
        // hidden in the scratchpad, then floated for the recall.
        let rules = WindowRules {
            floating: Some(true),
            scratchpad: Some(true),
            workspace: Some(2),
        };
        assert_eq!(
            evaluate_rule_effects(&rules),
            vec![
                RuleEffect::MoveToWorkspace(2),
                RuleEffect::Scratchpad,
                RuleEffect::Float,
            ]
        );
    }

    #[test]
    fn workspace_numbers_stay_one_based() {
        // 1 is the first workspace, 0 is invalid (both saturate to it).
        let rules = |ws| WindowRules {
            floating: None,
            scratchpad: None,
            workspace: Some(ws),
        };
        assert_eq!(
            evaluate_rule_effects(&rules(1)),
            vec![RuleEffect::MoveToWorkspace(1)]
        );
        assert_eq!(
            evaluate_rule_effects(&rules(0)),
            vec![RuleEffect::MoveToWorkspace(0)]
        );
    }
}
