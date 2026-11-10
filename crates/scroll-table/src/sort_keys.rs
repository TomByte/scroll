//! Editing a view's sort: the list of keys the sort editor and column menu change.

use scroll_core::id::PropertyId;
use scroll_core::record::{SortDirection, SortKey};

/// Adds an ascending key for `property` at the end, unless it is already sorted on.
pub fn with_key_added(keys: &[SortKey], property: PropertyId) -> Vec<SortKey> {
    let mut keys = keys.to_vec();
    if !keys.iter().any(|key| key.property == property) {
        keys.push(SortKey {
            property,
            direction: SortDirection::Ascending,
        });
    }
    keys
}

pub fn with_direction_flipped(keys: &[SortKey], index: usize) -> Vec<SortKey> {
    let mut keys = keys.to_vec();
    if let Some(key) = keys.get_mut(index) {
        key.direction = match key.direction {
            SortDirection::Ascending => SortDirection::Descending,
            SortDirection::Descending => SortDirection::Ascending,
        };
    }
    keys
}

pub fn without_key(keys: &[SortKey], index: usize) -> Vec<SortKey> {
    keys.iter()
        .enumerate()
        .filter(|(position, _)| *position != index)
        .map(|(_, key)| *key)
        .collect()
}

/// The keys after dragging the one at `from` to where the one at `to` is.
pub fn with_key_moved(keys: &[SortKey], from: usize, to: usize) -> Vec<SortKey> {
    let mut keys = keys.to_vec();
    if from < keys.len() && to < keys.len() {
        let key = keys.remove(from);
        keys.insert(to, key);
    }
    keys
}

/// Sorting from a column header replaces the whole sort with that one key.
pub fn only_by(property: PropertyId, direction: SortDirection) -> Vec<SortKey> {
    vec![SortKey {
        property,
        direction,
    }]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keys_are_added_once_flipped_and_removed() {
        let (first, second) = (PropertyId::generate(), PropertyId::generate());
        let keys = with_key_added(&with_key_added(&[], first), second);
        assert_eq!(with_key_added(&keys, first), keys);
        let flipped = with_direction_flipped(&keys, 1);
        assert_eq!(flipped[1].direction, SortDirection::Descending);
        assert_eq!(flipped[0].direction, SortDirection::Ascending);
        assert_eq!(without_key(&flipped, 0), vec![flipped[1]]);
        assert_eq!(with_direction_flipped(&keys, 9), keys);
        assert_eq!(only_by(first, SortDirection::Descending).len(), 1);
    }

    #[test]
    fn keys_can_be_reordered() {
        let ids: Vec<PropertyId> = (0..3).map(|_| PropertyId::generate()).collect();
        let keys = ids
            .iter()
            .fold(Vec::new(), |keys, id| with_key_added(&keys, *id));
        let moved = with_key_moved(&keys, 2, 0);
        assert_eq!(
            moved.iter().map(|key| key.property).collect::<Vec<_>>(),
            vec![ids[2], ids[0], ids[1]]
        );
        assert_eq!(with_key_moved(&keys, 0, 7), keys);
    }
}
