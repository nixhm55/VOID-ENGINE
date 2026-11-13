use crate::native_bridge::{MenuDefinition, UserEvent};
use muda::{Menu, MenuEvent, MenuItem, PredefinedMenuItem, Submenu};
use std::collections::HashMap;
use tao::event_loop::EventLoopProxy;

const INSPECT_ELEMENT_ID: &str = "void-inspect-element";

pub(crate) struct NativeMenu {
    root: Menu,
    dynamic_menus: Vec<Submenu>,
    dynamic_actions: HashMap<String, String>,
    dynamic_origin: Option<String>,
    definitions: Vec<MenuDefinition>,
    generation: u64,
}

impl NativeMenu {
    pub(crate) fn new(app_name: &str, proxy: EventLoopProxy<UserEvent>) -> muda::Result<Self> {
        let root = Menu::new();

        let app_menu = Submenu::new(app_name, true);
        app_menu.append(&PredefinedMenuItem::about(None, None))?;
        app_menu.append(&PredefinedMenuItem::separator())?;
        app_menu.append(&PredefinedMenuItem::quit(None))?;
        root.append(&app_menu)?;

        let file_menu = Submenu::new("File", true);
        file_menu.append(&PredefinedMenuItem::close_window(None))?;
        root.append(&file_menu)?;

        let edit_menu = Submenu::new("Edit", true);
        edit_menu.append(&PredefinedMenuItem::undo(None))?;
        edit_menu.append(&PredefinedMenuItem::redo(None))?;
        edit_menu.append(&PredefinedMenuItem::separator())?;
        edit_menu.append(&PredefinedMenuItem::cut(None))?;
        edit_menu.append(&PredefinedMenuItem::copy(None))?;
        edit_menu.append(&PredefinedMenuItem::paste(None))?;
        edit_menu.append(&PredefinedMenuItem::separator())?;
        edit_menu.append(&PredefinedMenuItem::select_all(None))?;
        root.append(&edit_menu)?;

        let view_menu = Submenu::new("View", true);
        view_menu.append(&PredefinedMenuItem::actual_size(None))?;
        view_menu.append(&PredefinedMenuItem::zoom_in(None))?;
        view_menu.append(&PredefinedMenuItem::zoom_out(None))?;
        view_menu.append(&PredefinedMenuItem::separator())?;
        view_menu.append(&PredefinedMenuItem::fullscreen(None))?;
        root.append(&view_menu)?;

        let window_menu = Submenu::new("Window", true);
        window_menu.append(&PredefinedMenuItem::minimize(None))?;
        window_menu.append(&PredefinedMenuItem::maximize(None))?;
        window_menu.append(&PredefinedMenuItem::separator())?;
        window_menu.append(&PredefinedMenuItem::bring_all_to_front(None))?;
        root.append(&window_menu)?;

        let developer_menu = Submenu::new("Developer", true);
        developer_menu.append(&MenuItem::with_id(
            INSPECT_ELEMENT_ID,
            "Inspect Element",
            true,
            None,
        ))?;
        root.append(&developer_menu)?;

        let help_menu = Submenu::new("Help", true);
        root.append(&help_menu)?;

        root.init_for_nsapp();
        window_menu.set_as_windows_menu_for_nsapp();
        help_menu.set_as_help_menu_for_nsapp();

        MenuEvent::set_event_handler(Some(move |event: MenuEvent| {
            let id = event.id.0;
            let user_event = if id == INSPECT_ELEMENT_ID {
                UserEvent::InspectElement
            } else {
                UserEvent::MenuAction { id }
            };
            let _ = proxy.send_event(user_event);
        }));

        Ok(Self {
            root,
            dynamic_menus: Vec::new(),
            dynamic_actions: HashMap::new(),
            dynamic_origin: None,
            definitions: Vec::new(),
            generation: 0,
        })
    }

    pub(crate) fn update(
        &mut self,
        origin: String,
        menus: Vec<MenuDefinition>,
    ) -> muda::Result<()> {
        if self.dynamic_origin.as_deref() == Some(origin.as_str()) && self.definitions == menus {
            return Ok(());
        }

        let generation = self.generation;
        let next_definitions = menus.clone();
        let mut next_menus = Vec::with_capacity(menus.len());
        let mut next_actions = HashMap::new();

        for (menu_index, definition) in menus.into_iter().enumerate() {
            let submenu = Submenu::new(definition.title, true);
            for (item_index, item) in definition.items.into_iter().enumerate() {
                let native_id = format!("void-dynamic-{generation}-{menu_index}-{item_index}");
                next_actions.insert(native_id.clone(), item.id);
                submenu.append(&MenuItem::with_id(native_id, item.label, true, None))?;
            }
            next_menus.push(submenu);
        }

        self.clear_dynamic()?;
        let mut inserted = 0;
        for (index, submenu) in next_menus.iter().enumerate() {
            if let Err(error) = self.root.insert(submenu, index + 5) {
                for added in next_menus[..inserted].iter().rev() {
                    if let Err(cleanup_error) = self.root.remove(added) {
                        eprintln!(
                            "Could not roll back a partial native menu update: {cleanup_error}"
                        );
                    }
                }
                return Err(error);
            }
            inserted += 1;
        }

        self.dynamic_menus = next_menus;
        self.dynamic_actions = next_actions;
        self.dynamic_origin = Some(origin);
        self.definitions = next_definitions;
        self.generation = self.generation.wrapping_add(1);
        Ok(())
    }

    pub(crate) fn clear_if_origin_mismatch(&mut self, current_origin: Option<&str>) {
        if self
            .dynamic_origin
            .as_deref()
            .is_some_and(|origin| Some(origin) != current_origin)
        {
            if let Err(error) = self.clear_dynamic() {
                eprintln!("Could not clear stale native menus: {error}");
            }
        }
    }

    pub(crate) fn action_for<'a>(
        &'a self,
        native_id: &str,
        current_origin: Option<&str>,
    ) -> Option<&'a str> {
        if self.dynamic_origin.as_deref() != current_origin {
            return None;
        }
        self.dynamic_actions.get(native_id).map(String::as_str)
    }

    fn clear_dynamic(&mut self) -> muda::Result<()> {
        for submenu in &self.dynamic_menus {
            self.root.remove(submenu)?;
        }
        self.dynamic_menus.clear();
        self.dynamic_actions.clear();
        self.dynamic_origin = None;
        self.definitions.clear();
        Ok(())
    }
}
