//! The records the window shows, loaded on demand and never the whole workspace: the loaded part
//! of the page tree, the databases hosted by loaded pages and their views, property values and
//! tags of opened pages, and every tag with its page count (tags are few). Local changes are
//! applied here as `Transaction`s at once, the same ones the store applies, so the UI never
//! waits; changes from elsewhere arrive as fresh copies of just the records that changed.

use scroll_core::validation::tag_name_key;
use std::collections::{BTreeSet, HashMap};

use scroll_core::id::{DatabaseId, PageId, PropertyId, SelectOptionId, TagId, ViewId};
use scroll_core::operation::{FieldChange, Operation, Record, RecordId, Transaction};
use scroll_core::record::{
    Database, Page, PageTag, Property, PropertyType, PropertyValue, SelectOption, Tag, TagColor,
    View, ViewType,
};
use scroll_core::store::TagUsage;

use super::page_tree::{MoveError, PageTree};

#[derive(Clone, Debug, Default)]
pub struct WorkspaceRecords {
    pub tree: PageTree,
    databases: HashMap<DatabaseId, Database>,
    views: HashMap<ViewId, View>,
    values: HashMap<(PageId, PropertyId), PropertyValue>,
    /// Which loaded database each loaded page hosts.
    hosts: HashMap<PageId, DatabaseId>,
    tags: HashMap<TagId, Tag>,
    /// Live pages carrying each tag, from the store, kept up to date by local changes.
    tag_counts: HashMap<TagId, usize>,
    /// Which opened pages carry which tags, as (page, tag).
    page_tags: BTreeSet<(PageId, TagId)>,
}

pub type DatabaseWithViews = (Database, Vec<View>);

/// One parent's children as read from the store, with the databases they host.
#[derive(Debug, Default)]
pub struct ChildrenBatch {
    pub parent: Option<PageId>,
    /// Each child with its own count of children.
    pub children: Vec<(Page, usize)>,
    pub databases: Vec<DatabaseWithViews>,
}

/// What showing one page needs: the page and its ancestors (for breadcrumbs), the database it
/// hosts or is a row of, its property values and its tags.
#[derive(Debug)]
pub struct PageDetails {
    pub page: PageId,
    /// The page first, then each ancestor; empty if the page is gone.
    pub chain: Vec<Page>,
    pub databases: Vec<DatabaseWithViews>,
    pub values: Vec<(PropertyId, PropertyValue)>,
    pub tags: Vec<Tag>,
}

/// Fresh copies of records changed elsewhere (another device, a table's own connection).
#[derive(Debug, Default)]
pub struct ChangedRecords {
    /// Each changed page as stored now with its child count; `None` if it was deleted.
    pub pages: Vec<(PageId, Option<(Page, usize)>)>,
    pub databases: Vec<(DatabaseId, Option<DatabaseWithViews>)>,
    /// Values of changed pages that are open, replacing what was loaded.
    pub values: Vec<(PageId, Vec<(PropertyId, PropertyValue)>)>,
    /// Every tag, when tags or memberships changed.
    pub tags: Option<Vec<TagUsage>>,
    /// Tags of changed pages that are open.
    pub page_tags: Vec<(PageId, Vec<Tag>)>,
}

impl WorkspaceRecords {
    /// Takes in the first batch of a parent's children read from the store, replacing what
    /// was loaded.
    pub fn load_children(&mut self, batch: ChildrenBatch) {
        self.load_databases(batch.databases);
        self.tree.load_children(batch.parent, batch.children);
    }

    /// Takes in a later batch of a parent's children, after those already loaded.
    pub fn append_children(&mut self, batch: ChildrenBatch) {
        self.load_databases(batch.databases);
        self.tree.append_children(batch.parent, batch.children);
    }

    /// Takes in what showing a page needs.
    pub fn load_page_details(&mut self, details: PageDetails) {
        self.tree.insert_pages(details.chain);
        self.load_databases(details.databases);
        self.values.retain(|(page, _), _| *page != details.page);
        self.values.extend(
            details
                .values
                .into_iter()
                .map(|(property, value)| ((details.page, property), value)),
        );
        self.replace_page_tags(details.page, details.tags);
    }

    pub fn load_tags(&mut self, tags: Vec<TagUsage>) {
        self.tags.clear();
        self.tag_counts.clear();
        for usage in tags {
            self.tag_counts.insert(usage.tag.id, usage.page_count);
            self.tags.insert(usage.tag.id, usage.tag);
        }
    }

    fn replace_page_tags(&mut self, page: PageId, tags: Vec<Tag>) {
        self.page_tags.retain(|(member, _)| *member != page);
        for tag in tags {
            self.page_tags.insert((page, tag.id));
            self.tags.entry(tag.id).or_insert(tag);
        }
    }

    fn load_databases(&mut self, databases: Vec<DatabaseWithViews>) {
        for (database, views) in databases {
            self.views.retain(|_, view| view.database != database.id);
            self.views
                .extend(views.into_iter().map(|view| (view.id, view)));
            self.hosts.insert(database.host_page, database.id);
            self.databases.insert(database.id, database);
        }
    }

    /// Takes in records changed elsewhere. Pages that are neither loaded nor listed under a
    /// loaded parent are skipped, so a change to thousands of rows nobody is looking at costs
    /// one lookup each.
    pub fn apply_changes(&mut self, changes: ChangedRecords) {
        for (id, page) in changes.pages {
            match page {
                Some((page, child_count)) => {
                    let is_shown = self.tree.page(id).is_some()
                        || (page.row_of.is_none() && self.tree.are_children_loaded(page.parent));
                    if is_shown {
                        self.tree.upsert(page);
                        self.tree.set_child_count(id, child_count);
                    }
                }
                None => self.tree.remove(id),
            }
        }
        for (id, database) in changes.databases {
            match database {
                Some(database) => self.load_databases(vec![database]),
                None => {
                    if let Some(database) = self.databases.remove(&id) {
                        self.hosts.remove(&database.host_page);
                    }
                    self.views.retain(|_, view| view.database != id);
                }
            }
        }
        for (page, values) in changes.values {
            self.values.retain(|(member, _), _| *member != page);
            self.values.extend(
                values
                    .into_iter()
                    .map(|(property, value)| ((page, property), value)),
            );
        }
        if let Some(tags) = changes.tags {
            self.load_tags(tags);
        }
        for (page, tags) in changes.page_tags {
            self.replace_page_tags(page, tags);
        }
    }

    /// Databases whose records are loaded, so a change to one can be re-read.
    pub fn loaded_databases(&self) -> Vec<DatabaseId> {
        self.databases.keys().copied().collect()
    }

    pub fn database_hosted_by(&self, page: PageId) -> Option<&Database> {
        self.databases.get(self.hosts.get(&page)?)
    }

    pub fn database(&self, id: DatabaseId) -> Option<&Database> {
        self.databases.get(&id)
    }

    /// The database's views in creation order (view ids are time-ordered).
    pub fn views_of(&self, database: DatabaseId) -> Vec<&View> {
        let mut views: Vec<&View> = self
            .views
            .values()
            .filter(|view| view.database == database)
            .collect();
        views.sort_by_key(|view| view.id);
        views
    }

    pub fn value(&self, page: PageId, property: PropertyId) -> Option<&PropertyValue> {
        self.values.get(&(page, property))
    }

    pub fn apply(&mut self, transaction: &Transaction) {
        self.tree.apply(transaction);
        for operation in &transaction.operations {
            match operation {
                Operation::Create(Record::Database(database)) => {
                    self.hosts.insert(database.host_page, database.id);
                    self.databases.insert(database.id, database.clone());
                }
                Operation::Create(Record::View(view)) => {
                    self.views.insert(view.id, view.clone());
                }
                Operation::Set(FieldChange::Database { id, field }) => {
                    if let Some(database) = self.databases.get_mut(id) {
                        field.apply_to(database);
                    }
                }
                Operation::Set(FieldChange::View { id, field }) => {
                    if let Some(view) = self.views.get_mut(id) {
                        field.apply_to(view);
                    }
                }
                Operation::Set(FieldChange::PropertyValue {
                    page,
                    property,
                    value,
                }) => match value {
                    Some(value) => {
                        self.values.insert((*page, *property), value.clone());
                    }
                    None => {
                        self.values.remove(&(*page, *property));
                    }
                },
                Operation::Delete(RecordId::Database(id)) => {
                    if let Some(database) = self.databases.remove(id) {
                        self.hosts.remove(&database.host_page);
                    }
                }
                Operation::Delete(RecordId::View(id)) => {
                    self.views.remove(id);
                }
                Operation::Create(Record::Tag(tag)) => {
                    self.tags.insert(tag.id, tag.clone());
                }
                Operation::Set(FieldChange::Tag { id, field }) => {
                    if let Some(tag) = self.tags.get_mut(id) {
                        field.apply_to(tag);
                    }
                }
                Operation::Delete(RecordId::Tag(id)) => {
                    self.tags.remove(id);
                    self.tag_counts.remove(id);
                    self.page_tags.retain(|(_, tag)| tag != id);
                }
                Operation::Create(Record::PageTag(membership)) => {
                    if self.page_tags.insert((membership.page, membership.tag)) {
                        *self.tag_counts.entry(membership.tag).or_default() += 1;
                    }
                }
                Operation::Delete(RecordId::PageTag { page, tag }) => {
                    if self.page_tags.remove(&(*page, *tag))
                        && let Some(count) = self.tag_counts.get_mut(tag)
                    {
                        *count = count.saturating_sub(1);
                    }
                }
                // Pages are the tree's, applied above.
                Operation::Create(Record::Page(_))
                | Operation::Set(FieldChange::Page { .. })
                | Operation::Delete(RecordId::Page(_)) => {}
            }
        }
    }

    /// The page's tags, by name ignoring case.
    pub fn tags_of(&self, page: PageId) -> Vec<&Tag> {
        let mut tags: Vec<&Tag> = self
            .page_tags
            .range((page, TagId(uuid::Uuid::nil()))..)
            .take_while(|(member, _)| *member == page)
            .filter_map(|(_, tag)| self.tags.get(tag))
            .collect();
        tags.sort_by_key(|tag| tag_name_key(&tag.name));
        tags
    }

    /// Every tag with how many live pages carry it, by name ignoring case.
    pub fn tags_with_counts(&self) -> Vec<(&Tag, usize)> {
        let mut tags: Vec<(&Tag, usize)> = self
            .tags
            .values()
            .map(|tag| (tag, self.tag_counts.get(&tag.id).copied().unwrap_or(0)))
            .collect();
        tags.sort_by_key(|(tag, _)| tag_name_key(&tag.name));
        tags
    }

    /// The tag named `name`, ignoring case and surrounding spaces.
    pub fn tag_named(&self, name: &str) -> Option<&Tag> {
        let wanted = tag_name_key(name);
        self.tags
            .values()
            .find(|tag| tag_name_key(&tag.name) == wanted)
    }

    /// Tags `page` with `name`, creating the tag first if none has that name.
    pub fn add_tag(&self, page: PageId, name: &str, color: TagColor) -> Transaction {
        let mut operations = Vec::new();
        let tag = match self.tag_named(name) {
            Some(tag) => tag.id,
            None => {
                let tag = Tag {
                    id: TagId::generate(),
                    name: name.trim().to_string(),
                    color,
                };
                let id = tag.id;
                operations.push(Operation::Create(Record::Tag(tag)));
                id
            }
        };
        if !self.page_tags.contains(&(page, tag)) {
            operations.push(Operation::Create(Record::PageTag(PageTag { page, tag })));
        }
        Transaction::new(operations)
    }

    /// Keeps a tag read from the store, e.g. one created on another device.
    pub fn remember_tag(&mut self, tag: Tag) {
        self.tags.entry(tag.id).or_insert(tag);
    }

    /// A new page under `parent` hosting a database with a starter schema (status, tags and
    /// a due date), one table view showing every property, and a few empty rows.
    pub fn create_database(
        &self,
        parent: Option<PageId>,
    ) -> Result<(PageId, Transaction), MoveError> {
        let (host, create_host) = self.tree.create_page(parent, "")?;
        let properties = starter_properties();
        let database = Database {
            id: DatabaseId::generate(),
            host_page: host,
            properties: properties.clone(),
        };
        let view = new_view(database.id, "Table", &properties);
        let mut operations = create_host.operations;
        operations.push(Operation::Create(Record::Database(database.clone())));
        operations.push(Operation::Create(Record::View(view)));
        Ok((host, Transaction::new(operations)))
    }

    /// A new table view of `database` named `name`, showing every property.
    pub fn add_view(&self, database: DatabaseId, name: &str) -> Option<(ViewId, Transaction)> {
        let properties = &self.databases.get(&database)?.properties;
        let view = new_view(database, name, properties);
        let id = view.id;
        Some((
            id,
            Transaction::new(vec![Operation::Create(Record::View(view))]),
        ))
    }
}

fn new_view(database: DatabaseId, name: &str, properties: &[Property]) -> View {
    View {
        id: ViewId::generate(),
        database,
        name: name.to_string(),
        view_type: ViewType::Table,
        filter: Vec::new(),
        sort: Vec::new(),
        visible_columns: properties.iter().map(|property| property.id).collect(),
        column_widths: Vec::new(),
    }
}

fn starter_properties() -> Vec<Property> {
    let options = |names: &[&str]| -> Vec<SelectOption> {
        names
            .iter()
            .map(|name| SelectOption {
                id: SelectOptionId::generate(),
                name: (*name).to_string(),
            })
            .collect()
    };
    let property = |name: &str, property_type, options| Property {
        id: PropertyId::generate(),
        name: name.to_string(),
        property_type,
        options,
    };
    vec![
        property(
            "Status",
            PropertyType::Select,
            options(&["Not started", "In progress", "Done"]),
        ),
        property(
            "Tags",
            PropertyType::MultiSelect,
            options(&["Idea", "Important"]),
        ),
        property("Due", PropertyType::Date, Vec::new()),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_new_database_has_a_host_page_and_a_view() {
        let mut records = WorkspaceRecords::default();
        let (host, transaction) = records.create_database(None).unwrap();
        records.apply(&transaction);
        let database = records.database_hosted_by(host).unwrap().clone();
        assert_eq!(database.properties.len(), 3);
        let views = records.views_of(database.id);
        assert_eq!(views.len(), 1);
        assert_eq!(views[0].visible_columns.len(), 3);
        assert_eq!(records.tree.children(None).len(), 1);
        assert!(records.tree.is_live(host));
    }

    #[test]
    fn views_can_be_added_renamed_and_deleted() {
        let mut records = WorkspaceRecords::default();
        let (host, transaction) = records.create_database(None).unwrap();
        records.apply(&transaction);
        let database = records.database_hosted_by(host).unwrap().id;
        let (second, transaction) = records.add_view(database, "Board-ish").unwrap();
        records.apply(&transaction);
        records.apply(&Transaction::rename_view(second, "Mine"));
        let names: Vec<String> = records
            .views_of(database)
            .iter()
            .map(|view| view.name.clone())
            .collect();
        assert_eq!(names, vec!["Table".to_string(), "Mine".to_string()]);
        records.apply(&Transaction::delete(RecordId::View(second)));
        assert_eq!(records.views_of(database).len(), 1);
    }

    #[test]
    fn property_values_set_and_clear() {
        let mut records = WorkspaceRecords::default();
        let (host, transaction) = records.create_database(None).unwrap();
        records.apply(&transaction);
        let status = records.database_hosted_by(host).unwrap().properties[0].clone();
        let row = PageId::generate();
        let done = PropertyValue::Select(status.options[2].id);
        records.apply(&Transaction::set_value(row, status.id, Some(done.clone())));
        assert_eq!(records.value(row, status.id), Some(&done));
        records.apply(&Transaction::set_value(row, status.id, None));
        assert_eq!(records.value(row, status.id), None);
    }
}
