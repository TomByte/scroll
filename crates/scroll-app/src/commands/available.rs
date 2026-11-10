//! When commands make sense, for their `is_available`.

use super::{PageKind, Situation};

pub fn always(_: &Situation) -> bool {
    true
}

pub fn never_in_palette(_: &Situation) -> bool {
    false
}

pub fn has_page(situation: &Situation) -> bool {
    situation.page.is_some()
}

pub fn in_document(situation: &Situation) -> bool {
    matches!(situation.page, Some(PageKind::Document | PageKind::Row))
}

pub fn in_database(situation: &Situation) -> bool {
    situation.page == Some(PageKind::Database)
}

pub fn is_signed_in(situation: &Situation) -> bool {
    situation.signed_in_servers > 0
}
