//! The sidebar and the places: its sections of catalogs, folding them and
//! the sidebar, and moving between the screens.

use std::rc::Rc;

use anchor::settings::Sidebar;
use slint::{ComponentHandle, ModelRc, VecModel};

use super::Session;
use crate::ui::{CatalogLink, Screen, SectionItem, Shell};

/// The place numbers the window sends.
pub(super) const HOME: i32 = 0;
pub(super) const SEARCH: i32 = 5;
pub(super) const SETTINGS: i32 = 6;

impl Session {
    /// Fills the sidebar: expanded or not, and the sections with their
    /// catalogs, folded as the user left them.
    pub(super) fn apply_sidebar(&self) {
        let Some(app) = self.app.upgrade() else {
            return;
        };
        let settings = self.settings.borrow();
        let state = self.state.borrow();
        let items: Vec<SectionItem> = state
            .sections
            .iter()
            .map(|section| SectionItem {
                title: section.title.as_str().into(),
                icon: match section.kind.as_str() {
                    "movie" => 0,
                    "series" => 1,
                    _ => 2,
                },
                folded: settings.folded.contains(&section.kind),
                catalogs: ModelRc::new(VecModel::from(
                    section
                        .catalogs
                        .iter()
                        .filter_map(|at| state.sources.catalog(*at))
                        .map(|(_, catalog)| CatalogLink {
                            title: catalog.title().into(),
                        })
                        .collect::<Vec<_>>(),
                )),
            })
            .collect();
        let shell = app.global::<Shell>();
        shell.set_sidebar_expanded(settings.sidebar == Sidebar::Expanded);
        shell.set_sections(ModelRc::new(VecModel::from(items)));
    }

    /// The fold button, or Ctrl+B: icons only, or back.
    pub(super) fn toggle_sidebar(&self) {
        {
            let mut settings = self.settings.borrow_mut();
            settings.sidebar = match settings.sidebar {
                Sidebar::Expanded => Sidebar::Collapsed,
                Sidebar::Collapsed => Sidebar::Expanded,
            };
        }
        self.save_settings();
        self.apply_sidebar();
    }

    /// A section's chevron: folds its catalogs away, or back.
    pub(super) fn fold(&self, section: i32) {
        let Some(kind) = usize::try_from(section)
            .ok()
            .and_then(|i| self.state.borrow().sections.get(i).map(|s| s.kind.clone()))
        else {
            return;
        };
        {
            let mut settings = self.settings.borrow_mut();
            if let Some(at) = settings.folded.iter().position(|k| *k == kind) {
                settings.folded.remove(at);
            } else {
                settings.folded.push(kind);
            }
        }
        self.save_settings();
        self.apply_sidebar();
    }

    /// Lights catalog `catalog` of section `section` in the sidebar.
    pub(super) fn light_catalog(&self, section: i32, catalog: i32) {
        if let Some(app) = self.app.upgrade() {
            let shell = app.global::<Shell>();
            shell.set_active_section(section);
            shell.set_active_catalog(catalog);
        }
    }

    /// A place: Home, the search field, Settings.
    pub(super) fn navigate(self: &Rc<Self>, place: i32) {
        let Some(app) = self.app.upgrade() else {
            return;
        };
        match place {
            HOME => {
                self.light_catalog(-1, -1);
                app.set_back_label("".into());
                self.show(Screen::Home);
            }
            SEARCH => app.invoke_focus_search(),
            SETTINGS => {}
            _ => {}
        }
    }

    /// The top bar's back button.
    pub(super) fn back(self: &Rc<Self>) {
        self.navigate(HOME);
    }
}
