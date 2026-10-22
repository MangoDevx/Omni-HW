//! Navigation layout and arrow-key page navigation.
//!
//! - Classic bar (one tab per page): Left/Right step through the pages in bar order, wrapping
//!   at the ends. Up/Down are left alone so pages can use them.
//! - Sidebar and Tabs (pages sorted into categories: down the left side, or category tabs with
//!   the open category's pages on a second row): Left/Right step through the top-level
//!   categories (wrapping), opening the page last used in a category; Up/Down step through the
//!   pages inside the current category, stopping at the ends.

use crate::app::Tab;
use crate::settings::NavStyle;

/// Multi-page categories, shown as sidebar headings and as Tabs category tabs: (label, pages).
pub const GROUPS: [(&str, &[Tab]); 3] = [
    (
        "Hardware",
        &[
            Tab::Cpu,
            Tab::Caches,
            Tab::Mainboard,
            Tab::Memory,
            Tab::Gpu,
            Tab::Storage,
            Tab::Power,
        ],
    ),
    ("Network", &[Tab::Network, Tab::LanSecurity]),
    (
        "Monitoring",
        &[Tab::Sensors, Tab::Recording, Tab::Alerts, Tab::EventLog],
    ),
];

/// Top-level entries that hold pages, in arrow-key order: the sidebar from top to bottom, or the
/// Tabs bar's five category tabs, then About (under Info) and Settings on the right. Record and
/// Report are actions rather than pages, so the arrows skip them.
pub const CATEGORIES: [&[Tab]; 7] = [
    &[Tab::Overview],
    &[Tab::Health],
    GROUPS[0].1,
    GROUPS[1].1,
    GROUPS[2].1,
    &[Tab::About],
    &[Tab::Settings],
];

/// Pages in the order the classic bar shows them (Settings sits at the right end).
fn classic_order() -> impl Iterator<Item = Tab> {
    Tab::ALL.into_iter().chain([Tab::Settings])
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Arrow {
    Left,
    Right,
    Up,
    Down,
}

impl Arrow {
    pub const ALL: [(egui::Key, Arrow); 4] = [
        (egui::Key::ArrowLeft, Arrow::Left),
        (egui::Key::ArrowRight, Arrow::Right),
        (egui::Key::ArrowUp, Arrow::Up),
        (egui::Key::ArrowDown, Arrow::Down),
    ];
}

/// Number of [`CATEGORIES`] drawn as tabs in the Tabs bar (About and Settings live with the
/// actions).
pub const TAB_CATEGORIES: usize = 5;

/// Tab label of a category: the group name, or the page name for a single-page category.
pub fn category_label(category: usize) -> &'static str {
    let pages = CATEGORIES[category];
    GROUPS
        .iter()
        .find(|(_, g)| *g == pages)
        .map_or_else(|| pages[0].name(), |&(label, _)| label)
}

/// Index into [`CATEGORIES`] of the category holding `tab`.
pub fn category_of(tab: Tab) -> Option<usize> {
    CATEGORIES.iter().position(|pages| pages.contains(&tab))
}

/// Remembers the page last shown in each category, so Left/Right return to it.
#[derive(Default)]
pub struct NavMemory {
    last: [Option<Tab>; CATEGORIES.len()],
}

impl NavMemory {
    pub fn visit(&mut self, tab: Tab) {
        if let Some(c) = category_of(tab) {
            self.last[c] = Some(tab);
        }
    }

    /// The page last shown in `category`, or its first page.
    pub fn page_in(&self, category: usize) -> Option<Tab> {
        self.last[category].or_else(|| CATEGORIES[category].first().copied())
    }
}

/// `i` moved by `delta` within `0..len`, wrapping around.
fn wrap(i: usize, delta: isize, len: usize) -> usize {
    (i as isize + delta).rem_euclid(len as isize) as usize
}

/// The page `arrow` moves to from `tab`, or `None` when it doesn't move (so the key can be left
/// for the page itself).
pub fn target(style: NavStyle, tab: Tab, arrow: Arrow, memory: &NavMemory) -> Option<Tab> {
    let delta = match arrow {
        Arrow::Left | Arrow::Up => -1,
        Arrow::Right | Arrow::Down => 1,
    };
    match (style, arrow) {
        (NavStyle::Classic, Arrow::Left | Arrow::Right) => {
            let pages: Vec<Tab> = classic_order().collect();
            let i = pages.iter().position(|&t| t == tab)?;
            Some(pages[wrap(i, delta, pages.len())])
        }
        (NavStyle::Classic, Arrow::Up | Arrow::Down) => None,
        (NavStyle::Sidebar | NavStyle::Tabs, Arrow::Left | Arrow::Right) => {
            let c = category_of(tab)?;
            memory.page_in(wrap(c, delta, CATEGORIES.len()))
        }
        (NavStyle::Sidebar | NavStyle::Tabs, Arrow::Up | Arrow::Down) => {
            let pages = CATEGORIES[category_of(tab)?];
            let i = pages.iter().position(|&t| t == tab)?;
            pages.get(i.checked_add_signed(delta)?).copied()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn go(style: NavStyle, tab: Tab, arrow: Arrow) -> Option<Tab> {
        target(style, tab, arrow, &NavMemory::default())
    }

    #[test]
    fn every_page_is_reachable() {
        for tab in classic_order() {
            assert!(category_of(tab).is_some(), "{tab:?} has no category");
        }
        assert_eq!(
            classic_order().count(),
            CATEGORIES.iter().map(|c| c.len()).sum()
        );
    }

    #[test]
    fn classic_left_right_walks_the_bar_and_wraps() {
        let c = NavStyle::Classic;
        assert_eq!(go(c, Tab::Overview, Arrow::Right), Some(Tab::Health));
        assert_eq!(go(c, Tab::Power, Arrow::Right), Some(Tab::Sensors));
        assert_eq!(go(c, Tab::Alerts, Arrow::Right), Some(Tab::EventLog));
        assert_eq!(go(c, Tab::EventLog, Arrow::Right), Some(Tab::About));
        assert_eq!(go(c, Tab::About, Arrow::Right), Some(Tab::Settings));
        assert_eq!(go(c, Tab::Settings, Arrow::Right), Some(Tab::Overview));
        assert_eq!(go(c, Tab::Overview, Arrow::Left), Some(Tab::Settings));
        assert_eq!(
            go(c, Tab::Cpu, Arrow::Down),
            None,
            "Up/Down left to the page"
        );
    }

    /// The two styles that sort pages into categories navigate the same way.
    const CATEGORY_STYLES: [NavStyle; 2] = [NavStyle::Sidebar, NavStyle::Tabs];

    #[test]
    fn categories_left_right_moves_between_categories() {
        for g in CATEGORY_STYLES {
            assert_eq!(go(g, Tab::Overview, Arrow::Right), Some(Tab::Health));
            assert_eq!(go(g, Tab::Health, Arrow::Right), Some(Tab::Cpu));
            assert_eq!(go(g, Tab::Gpu, Arrow::Right), Some(Tab::Network));
            assert_eq!(go(g, Tab::Network, Arrow::Right), Some(Tab::Sensors));
            assert_eq!(go(g, Tab::Alerts, Arrow::Right), Some(Tab::About));
            assert_eq!(go(g, Tab::EventLog, Arrow::Right), Some(Tab::About));
            assert_eq!(go(g, Tab::Settings, Arrow::Right), Some(Tab::Overview));
            assert_eq!(go(g, Tab::Overview, Arrow::Left), Some(Tab::Settings));
            assert_eq!(go(g, Tab::Sensors, Arrow::Left), Some(Tab::Network));
        }
    }

    #[test]
    fn categories_left_right_returns_to_the_last_page_of_a_category() {
        let mut m = NavMemory::default();
        m.visit(Tab::Recording);
        m.visit(Tab::About);
        // Left from About returns to the last page used in the Monitoring category.
        for style in CATEGORY_STYLES {
            assert_eq!(
                target(style, Tab::About, Arrow::Left, &m),
                Some(Tab::Recording)
            );
        }
        // The Tabs bar opens the same page when its Monitoring tab is clicked.
        assert_eq!(m.page_in(4), Some(Tab::Recording));
        assert_eq!(
            m.page_in(2),
            Some(Tab::Cpu),
            "first page when not visited yet"
        );
    }

    #[test]
    fn category_labels() {
        let labels: Vec<_> = (0..TAB_CATEGORIES).map(category_label).collect();
        assert_eq!(
            labels,
            [
                "Overview",
                "Health Check",
                "Hardware",
                "Network",
                "Monitoring"
            ]
        );
    }

    #[test]
    fn categories_up_down_moves_within_a_category() {
        for g in CATEGORY_STYLES {
            assert_eq!(go(g, Tab::Cpu, Arrow::Down), Some(Tab::Caches));
            assert_eq!(go(g, Tab::Caches, Arrow::Up), Some(Tab::Cpu));
            assert_eq!(go(g, Tab::Cpu, Arrow::Up), None, "stops at the top");
            assert_eq!(go(g, Tab::Power, Arrow::Down), None, "stops at the bottom");
            assert_eq!(go(g, Tab::Network, Arrow::Down), Some(Tab::LanSecurity));
            assert_eq!(go(g, Tab::LanSecurity, Arrow::Up), Some(Tab::Network));
            assert_eq!(go(g, Tab::Sensors, Arrow::Down), Some(Tab::Recording));
            assert_eq!(go(g, Tab::Alerts, Arrow::Down), Some(Tab::EventLog));
            assert_eq!(
                go(g, Tab::EventLog, Arrow::Down),
                None,
                "stops at the bottom"
            );
            assert_eq!(
                go(g, Tab::Overview, Arrow::Down),
                None,
                "single-page category"
            );
        }
    }
}
