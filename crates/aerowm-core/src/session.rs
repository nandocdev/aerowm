//! Persistent session state for hot restart.
//!
//! A [`SessionState`] is a plain-data snapshot of the window manager:
//! workspaces (names, layouts, stacking order, focus, floating geometry)
//! plus enough per-window identity to re-place windows as clients
//! reconnect after an in-place restart.
//!
//! [`WindowId`]s are process-local counters, so they are deliberately
//! *not* persisted. Windows are matched back by [`AppId`] (Wayland app_id
//! or X11 class) in arrival order instead.

use serde::{Deserialize, Serialize};

use crate::geometry::Rect;
use crate::layout::LayoutSpec;

/// Current session file format version. Bump when making an incompatible
/// change; loaders must reject unknown versions loudly instead of
/// mis-restoring state.
pub const SESSION_VERSION: u32 = 1;

/// Which backend a window came from. Matters for matching because a
/// Wayland app_id and an X11 class live in different namespaces.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum AppKind {
    Wayland,
    X11,
}

/// Stable-enough identity of a window for post-restart placement.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AppId {
    pub kind: AppKind,
    pub id: String,
    pub title: Option<String>,
}

impl AppId {
    pub fn wayland(app_id: impl Into<String>, title: Option<String>) -> Self {
        Self {
            kind: AppKind::Wayland,
            id: app_id.into(),
            title,
        }
    }

    pub fn x11(class: impl Into<String>, title: Option<String>) -> Self {
        Self {
            kind: AppKind::X11,
            id: class.into(),
            title,
        }
    }
}

/// One window entry inside a workspace snapshot.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct WindowEntry {
    /// Stacking order is the vector order.
    pub app: AppId,
    pub floating: bool,
    /// Pinned user geometry; meaningful when `floating` is set.
    pub geo: Option<Rect>,
    /// Whether this window held keyboard focus.
    pub focused: bool,
}

/// Serializable view of a single workspace.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct WorkspaceSnapshot {
    pub name: String,
    pub layout: LayoutSpec,
    /// Stacking order, oldest first.
    pub windows: Vec<WindowEntry>,
}

/// Whole-compositor snapshot, written to `session.json` on restart.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SessionState {
    pub version: u32,
    pub active_workspace: usize,
    pub workspaces: Vec<WorkspaceSnapshot>,
}

impl SessionState {
    /// Serializes to JSON for `session.json`.
    pub fn to_json(&self) -> Result<String, serde_json::Error> {
        serde_json::to_string_pretty(self)
    }

    /// Parses and validates a session file. Rejects unknown versions and
    /// clamps the active workspace index into range.
    pub fn from_json(raw: &str) -> Result<Self, SessionError> {
        let mut state: SessionState =
            serde_json::from_str(raw).map_err(SessionError::Malformed)?;
        if state.version != SESSION_VERSION {
            return Err(SessionError::UnsupportedVersion(state.version));
        }
        if state.workspaces.is_empty() {
            return Err(SessionError::Empty);
        }
        if state.active_workspace >= state.workspaces.len() {
            state.active_workspace = 0;
        }
        Ok(state)
    }

    /// Flattens snapshots into arrival-order placement directives.
    pub fn pending_placements(&self) -> Vec<PendingPlacement> {
        let mut out = Vec::new();
        for (ws_idx, ws) in self.workspaces.iter().enumerate() {
            for (position, entry) in ws.windows.iter().enumerate() {
                out.push(PendingPlacement {
                    app: entry.app.clone(),
                    workspace: ws_idx,
                    position,
                    floating: entry.floating,
                    geo: entry.geo,
                    focused: entry.focused,
                });
            }
        }
        out
    }
}

/// Placement directive consumed when a window with a matching [`AppId`]
/// maps after a restart. Entries are consumed first-match-first-served,
/// so two terminals restore into their recorded slots in arrival order.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PendingPlacement {
    pub app: AppId,
    pub workspace: usize,
    pub position: usize,
    pub floating: bool,
    pub geo: Option<Rect>,
    pub focused: bool,
}

#[derive(Debug)]
pub enum SessionError {
    Malformed(serde_json::Error),
    UnsupportedVersion(u32),
    Empty,
}

impl std::fmt::Display for SessionError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SessionError::Malformed(e) => write!(f, "malformed session file: {e}"),
            SessionError::UnsupportedVersion(v) => {
                write!(f, "unsupported session version {v} (expected {SESSION_VERSION})")
            }
            SessionError::Empty => write!(f, "session file has no workspaces"),
        }
    }
}

impl std::error::Error for SessionError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            SessionError::Malformed(e) => Some(e),
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> SessionState {
        SessionState {
            version: SESSION_VERSION,
            active_workspace: 1,
            workspaces: vec![
                WorkspaceSnapshot {
                    name: "1".into(),
                    layout: LayoutSpec::MonadTall {
                        master_ratio: 0.6,
                        master_count: 1,
                    },
                    windows: vec![WindowEntry {
                        app: AppId::wayland("kitty", Some("shell".into())),
                        floating: false,
                        geo: None,
                        focused: true,
                    }],
                },
                WorkspaceSnapshot {
                    name: "2".into(),
                    layout: LayoutSpec::Columns,
                    windows: vec![WindowEntry {
                        app: AppId::x11("Gimp", None),
                        floating: true,
                        geo: Some(Rect::new(100, 100, 800, 600)),
                        focused: false,
                    }],
                },
            ],
        }
    }

    #[test]
    fn session_json_roundtrip() {
        let state = sample();
        let json = state.to_json().expect("serialize");
        let back = SessionState::from_json(&json).expect("parse");
        assert_eq!(state, back);
    }

    #[test]
    fn rejects_bad_versions_and_empty() {
        let mut bad = sample();
        bad.version = 999;
        let json = bad.to_json().unwrap();
        assert!(matches!(
            SessionState::from_json(&json),
            Err(SessionError::UnsupportedVersion(999))
        ));

        assert!(matches!(
            SessionState::from_json("not json{{"),
            Err(SessionError::Malformed(_))
        ));

        let empty = SessionState {
            version: SESSION_VERSION,
            active_workspace: 0,
            workspaces: vec![],
        };
        assert!(matches!(
            SessionState::from_json(&empty.to_json().unwrap()),
            Err(SessionError::Empty)
        ));
    }

    #[test]
    fn clamps_active_workspace() {
        let mut state = sample();
        state.active_workspace = 99;
        let back = SessionState::from_json(&state.to_json().unwrap()).unwrap();
        assert_eq!(back.active_workspace, 0);
    }

    #[test]
    fn pending_placements_preserve_order() {
        let pending = sample().pending_placements();
        assert_eq!(pending.len(), 2);
        assert_eq!(pending[0].workspace, 0);
        assert_eq!(pending[0].position, 0);
        assert!(pending[0].focused);
        assert!(!pending[0].floating);
        assert_eq!(pending[1].workspace, 1);
        assert!(pending[1].floating);
        assert_eq!(pending[1].geo, Some(Rect::new(100, 100, 800, 600)));
    }

    #[test]
    fn layout_spec_roundtrip() {
        for spec in [
            LayoutSpec::MonadTall {
                master_ratio: 0.42,
                master_count: 2,
            },
            LayoutSpec::Columns,
            LayoutSpec::Max,
        ] {
            let json = serde_json::to_string(&spec).unwrap();
            let back: LayoutSpec = serde_json::from_str(&json).unwrap();
            assert_eq!(spec, back);
            // Every spec must instantiate a working layout.
            let area = Rect::new(0, 0, 800, 600);
            let _ = back.instantiate().apply(area, 3);
        }
        assert_eq!(
            LayoutSpec::from_name("nope"),
            None,
            "unknown names stay unknown"
        );
    }
}
