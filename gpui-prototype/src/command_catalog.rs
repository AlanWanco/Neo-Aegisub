use std::{collections::BTreeMap, sync::Arc};

use gpui_kit::component::menu::{PopupMenu, PopupMenuItem};
use gpui_kit::{Context, Window};
use serde::Deserialize;
use serde_json::json;

use crate::Workspace;

#[derive(Clone, Debug, Deserialize)]
pub struct CommandInfo {
    pub name: String,
    pub label: String,
    pub help: String,
    pub enabled: bool,
    pub active: bool,
    pub shortcut: String,
    pub surface: String,
}

impl CommandInfo {
    pub fn can_invoke(&self) -> bool {
        self.enabled && self.surface != "wx-control" && self.surface != "internal"
    }

    pub fn surface_label(&self) -> &'static str {
        match self.surface.as_str() {
            "native" => "GPUI 交互",
            "dialog" => "GPUI 面板",
            "wx-control" => "需旧版控件",
            "internal" => "内部入口",
            "automation" => "Lua Automation",
            _ => "共享项目操作",
        }
    }
}

#[derive(Clone, Debug, Default, Deserialize)]
pub struct MenuEntry {
    #[serde(default)]
    pub command: String,
    #[serde(default)]
    pub submenu: String,
    #[serde(default)]
    pub text: String,
    #[serde(default)]
    pub special: String,
    #[serde(default)]
    pub recent: String,
}

#[derive(Clone, Debug, Deserialize)]
pub struct Hotkey {
    pub command: String,
    pub context: String,
    pub key: String,
}

#[derive(Clone, Debug, Default, Deserialize)]
pub struct Catalog {
    pub commands: Vec<CommandInfo>,
    pub menus: BTreeMap<String, serde_json::Value>,
    #[serde(default)]
    pub bindings: Vec<Hotkey>,
    #[serde(default)]
    pub always_hotkeys: bool,
}

impl Catalog {
    pub fn command(&self, name: &str) -> Option<&CommandInfo> {
        self.commands.iter().find(|item| item.name == name)
    }

    pub fn shortcut(&self, context: &str, key: &str) -> Option<&CommandInfo> {
        let matching: Vec<_> = self
            .bindings
            .iter()
            .filter(|binding| binding.key == key)
            .collect();
        let command = if self.always_hotkeys {
            matching.iter().find(|binding| binding.context == "Always")
        } else {
            None
        }
        .or_else(|| {
            matching
                .iter()
                .rev()
                .find(|binding| binding.context == context)
        })
        .or_else(|| {
            matching
                .iter()
                .rev()
                .find(|binding| binding.context == "Default")
        })?;
        self.command(&command.command)
            .filter(|command| command.can_invoke())
    }

    /// First key bound to `command` in `context`, falling back to the global
    /// contexts the way the host resolves a keystroke.
    pub fn keys(&self, context: &str, command: &str) -> Option<&str> {
        self.bindings
            .iter()
            .find(|binding| binding.command == command && binding.context == context)
            .or_else(|| {
                self.bindings
                    .iter()
                    .find(|binding| binding.command == command && binding.context == "Always")
            })
            .or_else(|| {
                self.bindings
                    .iter()
                    .find(|binding| binding.command == command && binding.context == "Default")
            })
            .map(|binding| binding.key.as_str())
    }

    pub fn menu(&self, name: &str) -> Vec<MenuEntry> {
        self.menus
            .get(name)
            .and_then(|value| serde_json::from_value(value.clone()).ok())
            .unwrap_or_default()
    }
}

pub fn populate_menu(
    mut menu: PopupMenu,
    items: Vec<MenuEntry>,
    catalog: Arc<Catalog>,
    workspace: gpui_kit::WeakEntity<Workspace>,
    window: &mut Window,
    cx: &mut Context<PopupMenu>,
) -> PopupMenu {
    for item in items {
        if !item.command.is_empty() {
            if let Some(command) = catalog.command(&item.command) {
                let name = item.command.clone();
                let mut label = if item.text.is_empty() {
                    command.label.clone()
                } else {
                    item.text.replace('&', "")
                };
                if command.surface == "dialog" || command.surface == "wx-control" {
                    label.push_str(&format!(" · {}", command.surface_label()));
                }
                let weak = workspace.clone();
                menu = menu.item(
                    PopupMenuItem::new(label)
                        .disabled(!command.can_invoke())
                        .checked(command.active)
                        .on_click(move |_, _, cx| {
                            let _ = weak.update(cx, |view, cx| {
                                view.invoke(json!({"op":"command", "name":name}), cx);
                            });
                        }),
                );
            }
        } else if !item.submenu.is_empty()
            || !item.recent.is_empty()
            || item.special == "automation"
        {
            let children = if !item.submenu.is_empty() {
                catalog.menu(&item.submenu)
            } else if item.special == "automation" {
                catalog
                    .commands
                    .iter()
                    .filter(|command| {
                        command.name.starts_with("automation/lua/")
                            || matches!(
                                command.name.as_str(),
                                "am/manager" | "am/reload" | "am/reload/autoload"
                            )
                    })
                    .map(|command| MenuEntry {
                        command: command.name.clone(),
                        ..Default::default()
                    })
                    .collect()
            } else {
                let prefix = format!("recent/{}/", item.recent.to_lowercase());
                catalog
                    .commands
                    .iter()
                    .filter(|command| command.enabled && command.name.starts_with(&prefix))
                    .map(|command| MenuEntry {
                        command: command.name.clone(),
                        ..Default::default()
                    })
                    .collect()
            };
            let label = if item.text.is_empty() {
                "Recent".to_owned()
            } else {
                item.text.replace('&', "")
            };
            let catalog = catalog.clone();
            let weak = workspace.clone();
            menu = menu.submenu(label, window, cx, move |menu, window, cx| {
                populate_menu(
                    menu,
                    children.clone(),
                    catalog.clone(),
                    weak.clone(),
                    window,
                    cx,
                )
            });
        } else {
            menu = menu.separator();
        }
    }
    menu.scrollable(true).max_h(gpui_kit::px(580.))
}

/// Read validation after selecting the context row, so multi-line commands use
/// the same selection and enablement as the legacy grid menu.
pub fn subtitle_context_menu(
    menu: PopupMenu,
    id: i64,
    workspace: gpui_kit::WeakEntity<Workspace>,
    window: &mut Window,
    cx: &mut Context<PopupMenu>,
) -> PopupMenu {
    let Ok((bridge, draft, select)) = workspace.update(cx, |view, cx| {
        view.key_focus = crate::KeyFocus::Grid;
        view.grid_drag_anchor = None;
        view.follow_suppressed = true;
        let select = (!view.snapshot.selected.contains(&id))
            .then(|| json!({"op":"select", "id":id, "ids":[id]}));
        (view.bridge.clone(), view.pending_draft(cx), select)
    }) else {
        return menu;
    };
    cx.spawn_in(window, async move |menu, cx| {
        let result: anyhow::Result<_> = async {
            if let Some(draft) = draft {
                bridge.call(draft).await?;
            }
            if let Some(select) = select {
                bridge.call(select).await?;
            }
            let snapshot = bridge.snapshot().await?;
            let catalog: Catalog =
                serde_json::from_value(bridge.call(json!({"op":"catalog"})).await?)?;
            Ok((snapshot, Arc::new(catalog)))
        }
        .await;
        let _ = menu.update_in(cx, |menu, window, cx| match result {
            Ok((snapshot, catalog)) => {
                let _ = workspace.update(cx, |view, cx| {
                    view.apply_snapshot(snapshot, cx);
                    view.catalog = catalog.clone();
                    view.sync_display_mode(cx);
                });
                menu.rebuild(window, cx, |menu, window, cx| {
                    populate_menu(
                        menu,
                        catalog.menu("grid_context"),
                        catalog,
                        workspace,
                        window,
                        cx,
                    )
                });
            }
            Err(error) => {
                menu.rebuild(window, cx, |menu, _, _| menu.label("菜单读取失败"));
                let _ = workspace.update(cx, |view, cx| {
                    view.status = format!("菜单读取失败：{error}");
                    cx.notify();
                });
            }
        });
    })
    .detach();
    menu.label("读取菜单…")
}
