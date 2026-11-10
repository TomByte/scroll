use gpui::{App, Menu, MenuItem, SystemMenuType};

use crate::commands::{self, MenuName};

/// The menu bar, built from the command list.
pub(super) fn set_menus(cx: &mut App) {
    let menu = |name: &str, menu: MenuName| Menu {
        name: name.to_string().into(),
        items: commands::menu_items(crate::window::command_list::all(), menu),
    };
    let mut app_menu = menu("Scroll", MenuName::App);
    app_menu.items.insert(
        0,
        MenuItem::os_submenu("Services", SystemMenuType::Services),
    );
    app_menu.items.insert(1, MenuItem::separator());
    cx.set_menus(vec![
        app_menu,
        menu("File", MenuName::File),
        menu("Edit", MenuName::Edit),
        menu("Format", MenuName::Format),
        menu("View", MenuName::View),
        menu("Go", MenuName::Go),
    ]);
}
