//! Left-panel pages: which one surface the panel shows, and the switching rules.
//!
//! The v2 shell hosts one page at a time in the left panel (the Zed shape: one tool surface, not
//! a stack of sections), toggled from the status bar. The page state is plain data with a tested
//! `select`, so the status bar buttons cannot invent switching behavior of their own. The content
//! scan has a slot in the bar already - the conventions-and-integrity views are an engine concern
//! the editor will surface - but no page behind it yet, so it is disabled and `select` refuses it
//! rather than trusting every caller to check.

/// One left-panel page. `Scan` is the reserved slot: visible in the bar, not yet selectable.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Page {
    Scene,
    Prefabs,
    Scan,
}

impl Page {
    /// Every page in status-bar order.
    pub const ALL: [Page; 3] = [Page::Scene, Page::Prefabs, Page::Scan];

    /// Whether the page can be selected. The scan page is a reserved slot until the content scan
    /// exists.
    pub fn enabled(self) -> bool {
        !matches!(self, Page::Scan)
    }

    /// The status-bar tooltip.
    pub fn tooltip(self) -> &'static str {
        match self {
            Page::Scene => "Scene",
            Page::Prefabs => "Prefabs",
            Page::Scan => "Content scan (not built yet)",
        }
    }
}

/// Which page the left panel currently shows.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PageState {
    current: Page,
}

impl Default for PageState {
    fn default() -> PageState {
        PageState { current: Page::Scene }
    }
}

impl PageState {
    pub fn current(self) -> Page {
        self.current
    }

    /// Switch to `page`. Disabled pages are refused (the reserved scan slot), so the state can
    /// never land on a page with nothing behind it. Returns whether the switch happened.
    pub fn select(&mut self, page: Page) -> bool {
        if !page.enabled() {
            return false;
        }
        self.current = page;
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_disabled_scan_slot_is_refused() {
        let mut pages = PageState::default();
        pages.select(Page::Prefabs);
        assert!(!pages.select(Page::Scan), "the reserved slot must not be selectable");
        assert_eq!(pages.current(), Page::Prefabs, "a refused select changes nothing");
    }
}
