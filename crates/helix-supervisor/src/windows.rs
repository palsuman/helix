//! Testable Helix Host window policy (Task 2.4, REQ-ARCH-006).
//!
//! Native window creation stays in `main.rs`. This module decides whether to
//! focus an existing window, create another, restore a session, or shut the
//! kernel down when the last window closes.

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FolderOpenPlan {
    Focus { window_id: String },
    CreateNew,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RestorePlan {
    DefaultWindow,
    Restore {
        first_uses_existing_label: String,
        additional_ids: Vec<String>,
    },
}

pub fn folder_open_plan(existing_window_id: Option<&str>, force_new: bool) -> FolderOpenPlan {
    match existing_window_id {
        Some(window_id) if !force_new => FolderOpenPlan::Focus {
            window_id: window_id.to_string(),
        },
        _ => FolderOpenPlan::CreateNew,
    }
}

pub fn closing_last_window(remaining_after_close: u32) -> bool {
    remaining_after_close == 0
}

pub fn restore_plan(
    restore_enabled: bool,
    session_window_ids: &[String],
    existing_label: &str,
) -> RestorePlan {
    if !restore_enabled || session_window_ids.is_empty() {
        return RestorePlan::DefaultWindow;
    }
    RestorePlan::Restore {
        first_uses_existing_label: existing_label.to_string(),
        additional_ids: session_window_ids.iter().skip(1).cloned().collect(),
    }
}

pub fn restored_window_payload(record: &serde_json::Value, id: &str) -> serde_json::Value {
    serde_json::json!({
        "id": id,
        "roots": record.get("roots").cloned().unwrap_or_else(|| serde_json::json!([])),
        "geometry": record.get("geometry").cloned(),
        "layout": record.get("layout").cloned(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn opening_a_folder_focuses_the_existing_window_unless_duplicated() {
        assert_eq!(
            folder_open_plan(Some("main"), false),
            FolderOpenPlan::Focus {
                window_id: "main".into()
            }
        );
        assert_eq!(
            folder_open_plan(Some("main"), true),
            FolderOpenPlan::CreateNew
        );
        assert_eq!(folder_open_plan(None, false), FolderOpenPlan::CreateNew);
    }

    #[test]
    fn the_kernel_shuts_down_only_when_the_last_window_closes() {
        assert!(closing_last_window(0));
        assert!(!closing_last_window(1));
    }

    #[test]
    fn session_restore_reuses_the_boot_window_for_the_first_entry() {
        let ids = vec!["one".into(), "two".into()];
        assert_eq!(
            restore_plan(true, &ids, "main"),
            RestorePlan::Restore {
                first_uses_existing_label: "main".into(),
                additional_ids: vec!["two".into()],
            }
        );
        assert_eq!(
            restore_plan(false, &ids, "main"),
            RestorePlan::DefaultWindow
        );
    }

    #[test]
    fn session_restore_carries_geometry_and_layout_into_the_new_record() {
        let record = serde_json::json!({
            "id": "old-label",
            "roots": ["/workspace"],
            "geometry": { "x": 40, "y": 50, "width": 1200, "height": 800 },
            "layout": { "profiles": [{ "name": "Debug" }] }
        });

        let payload = restored_window_payload(&record, "main");

        assert_eq!(payload["id"], "main");
        assert_eq!(payload["roots"], serde_json::json!(["/workspace"]));
        assert_eq!(payload["geometry"]["x"], 40);
        assert_eq!(payload["layout"]["profiles"][0]["name"], "Debug");
    }
}
