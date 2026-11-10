//! Large workspaces for benchmarks: databases full of rows with realistic values, a deep page
//! tree with searchable text, and tags. Written straight into the tables in one transaction,
//! without validation or sync bookkeeping, so a million rows take seconds rather than minutes.
//! The workspace is local-only. Page text exists only in the search index (pages have no stored
//! content), which is what search and the sidebar read.
//!
//! The same `seed` and size always give the same workspace contents (ids differ: they are
//! time-ordered UUIDs).

use std::path::Path;

use rusqlite::Connection;

use crate::id::{DatabaseId, PageId, PropertyId, SelectOptionId, TagId, ViewId};
use crate::operation::{FieldChange, Record};
use crate::position::Position;
use crate::record::{
    Condition, Database, Date, FilterCondition, Page, PageTag, Property, PropertyType,
    PropertyValue, SelectOption, SortDirection, SortKey, Tag, TagColor, View, ViewType,
};
use crate::store::random::Random;
use crate::store::{StoreError, WorkspaceStore};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FixtureSize {
    /// The row count of each database; each has the ten properties `PROPERTY_NAMES` lists.
    pub database_rows: Vec<usize>,
    /// Ordinary pages in the tree (database host pages come on top).
    pub pages: usize,
    pub tags: usize,
}

impl FixtureSize {
    /// The scale target: 1,000,000 rows (one database of 500,000 and five of 100,000) and
    /// 100,000 pages.
    pub fn target() -> FixtureSize {
        FixtureSize {
            database_rows: vec![500_000, 100_000, 100_000, 100_000, 100_000, 100_000],
            pages: 100_000,
            tags: 40,
        }
    }

    /// One database of `rows` rows and a small tree around it.
    pub fn one_database(rows: usize) -> FixtureSize {
        FixtureSize {
            database_rows: vec![rows],
            pages: 1_000,
            tags: 40,
        }
    }
}

#[derive(Debug, Clone)]
pub struct Fixture {
    pub databases: Vec<FixtureDatabase>,
    pub tags: Vec<TagId>,
    /// Ordinary pages, in creation order.
    pub pages: Vec<PageId>,
}

#[derive(Debug, Clone)]
pub struct FixtureDatabase {
    pub database: Database,
    /// Every row, in position order.
    pub rows: Vec<PageId>,
    /// Shows every column, unsorted and unfiltered.
    pub all_rows_view: ViewId,
    /// Rows not done, by due date then estimate (largest first).
    pub open_by_due_view: ViewId,
}

pub const PROPERTY_NAMES: [&str; 10] = [
    "Status", "Priority", "Labels", "Estimate", "Due", "Done", "Owner", "Notes", "Link", "Points",
];

/// Builds the workspace in a new file at `path`, which must not exist yet.
pub fn generate_workspace(
    path: &Path,
    size: &FixtureSize,
    seed: u64,
) -> Result<Fixture, StoreError> {
    let mut store = WorkspaceStore::open(path)?;
    let tx = store.conn.transaction()?;
    let mut builder = Builder {
        conn: &tx,
        random: Random(seed),
    };
    let fixture = builder.build(size)?;
    tx.commit()?;
    Ok(fixture)
}

struct Builder<'conn> {
    conn: &'conn Connection,
    random: Random,
}

impl Builder<'_> {
    fn build(&mut self, size: &FixtureSize) -> Result<Fixture, StoreError> {
        let tags = self.create_tags(size.tags)?;
        let pages = self.create_page_tree(size.pages, &tags)?;
        let host_positions = positions(size.database_rows.len());
        let mut databases = Vec::new();
        for (index, (rows, position)) in size.database_rows.iter().zip(host_positions).enumerate() {
            databases.push(self.create_database(index, *rows, position, &tags)?);
        }
        Ok(Fixture {
            databases,
            tags,
            pages,
        })
    }

    fn create_tags(&mut self, count: usize) -> Result<Vec<TagId>, StoreError> {
        const COLORS: [TagColor; 9] = [
            TagColor::Gray,
            TagColor::Brown,
            TagColor::Orange,
            TagColor::Yellow,
            TagColor::Green,
            TagColor::Blue,
            TagColor::Purple,
            TagColor::Pink,
            TagColor::Red,
        ];
        let mut ids = Vec::with_capacity(count);
        for index in 0..count {
            let tag = Tag {
                id: TagId::generate(),
                name: format!("{} {index}", word(&mut self.random)),
                color: COLORS[index % COLORS.len()],
            };
            ids.push(tag.id);
            self.insert(&Record::Tag(tag))?;
        }
        Ok(ids)
    }

    /// A tree no more than five levels deep: about one page in a hundred is top-level, the rest
    /// go under a random earlier page that is not too deep.
    fn create_page_tree(
        &mut self,
        count: usize,
        tags: &[TagId],
    ) -> Result<Vec<PageId>, StoreError> {
        let mut pages: Vec<PageId> = Vec::with_capacity(count);
        let mut parents_with_room: Vec<(PageId, usize)> = Vec::new();
        let mut last_child_position: std::collections::HashMap<Option<PageId>, Position> =
            std::collections::HashMap::new();
        for _ in 0..count {
            let parent = if parents_with_room.is_empty() || self.random.with_probability(0.01) {
                None
            } else {
                Some(parents_with_room[self.random.below(parents_with_room.len())])
            };
            let depth = parent.map_or(0, |(_, depth)| depth + 1);
            let parent_id = parent.map(|(id, _)| id);
            let position = Position::between(last_child_position.get(&parent_id), None)
                .expect("there is always room after the last position");
            last_child_position.insert(parent_id, position.clone());
            let page = Page {
                id: PageId::generate(),
                parent: parent_id,
                title: words(&mut self.random, 1, 6),
                icon: None,
                position,
                row_of: None,
                is_archived: self.random.with_probability(0.02),
            };
            let id = page.id;
            self.insert(&Record::Page(page))?;
            if self.random.with_probability(0.4) {
                let body = words(&mut self.random, 20, 150);
                super::search::update_body(self.conn, id, &body)?;
            }
            self.tag_page(id, tags, 0.3)?;
            if depth < 4 {
                parents_with_room.push((id, depth));
            }
            pages.push(id);
        }
        Ok(pages)
    }

    fn tag_page(&mut self, page: PageId, tags: &[TagId], chance: f64) -> Result<(), StoreError> {
        if tags.is_empty() || !self.random.with_probability(chance) {
            return Ok(());
        }
        for _ in 0..1 + self.random.below(3) {
            let tag = tags[skewed_below(&mut self.random, tags.len())];
            self.insert(&Record::PageTag(PageTag { page, tag }))?;
        }
        Ok(())
    }

    fn create_database(
        &mut self,
        index: usize,
        row_count: usize,
        host_position: Position,
        tags: &[TagId],
    ) -> Result<FixtureDatabase, StoreError> {
        let host = Page {
            id: PageId::generate(),
            parent: None,
            title: format!("Database {index}"),
            icon: None,
            position: host_position,
            row_of: None,
            is_archived: false,
        };
        let database = Database {
            id: DatabaseId::generate(),
            host_page: host.id,
            properties: properties(),
        };
        self.insert(&Record::Page(host))?;
        self.insert(&Record::Database(database.clone()))?;
        let (all_rows_view, open_by_due_view) = views(&database);
        let view_ids = (all_rows_view.id, open_by_due_view.id);
        self.insert(&Record::View(all_rows_view))?;
        self.insert(&Record::View(open_by_due_view))?;

        let mut rows = Vec::with_capacity(row_count);
        for position in positions(row_count) {
            let row = Page {
                id: PageId::generate(),
                parent: Some(database.host_page),
                title: words(&mut self.random, 2, 6),
                icon: None,
                position,
                row_of: Some(database.id),
                is_archived: false,
            };
            let id = row.id;
            self.insert(&Record::Page(row))?;
            for property in &database.properties {
                if let Some(value) = self.value_for(property) {
                    let change = FieldChange::PropertyValue {
                        page: id,
                        property: property.id,
                        value: Some(value),
                    };
                    super::kinds::write_field(self.conn, &change)?;
                }
            }
            self.tag_page(id, tags, 0.05)?;
            rows.push(id);
        }
        Ok(FixtureDatabase {
            database,
            rows,
            all_rows_view: view_ids.0,
            open_by_due_view: view_ids.1,
        })
    }

    /// Each property's value distribution; `None` leaves the cell empty.
    fn value_for(&mut self, property: &Property) -> Option<PropertyValue> {
        let random = &mut self.random;
        let option = |random: &mut Random, weights: &[u32]| {
            SelectOptionId::clone(&property.options[weighted(random, weights)].id)
        };
        match property.name.as_str() {
            "Status" if random.with_probability(0.97) => Some(PropertyValue::Select(option(
                random,
                &[15, 25, 15, 5, 35, 5],
            ))),
            "Priority" if random.with_probability(0.8) => {
                Some(PropertyValue::Select(option(random, &[30, 40, 20, 10])))
            }
            "Labels" if random.with_probability(0.6) => {
                let mut labels: Vec<SelectOptionId> = (0..1 + random.below(3))
                    .map(|_| property.options[skewed_below(random, property.options.len())].id)
                    .collect();
                labels.dedup();
                Some(PropertyValue::MultiSelect(labels))
            }
            "Estimate" if random.with_probability(0.75) => {
                let sizes = [1.0, 2.0, 3.0, 5.0, 8.0, 13.0, 21.0, 0.5];
                Some(PropertyValue::Number(
                    sizes[skewed_below(random, sizes.len())],
                ))
            }
            "Due" if random.with_probability(0.7) => Some(PropertyValue::Date(date(random))),
            "Done" => match random.below(20) {
                0..7 => Some(PropertyValue::Checkbox(true)),
                7..15 => Some(PropertyValue::Checkbox(false)),
                _ => None,
            },
            "Owner" if random.with_probability(0.9) => Some(PropertyValue::Text(format!(
                "{} {}",
                word(random),
                word(random)
            ))),
            "Notes" if random.with_probability(0.4) => {
                Some(PropertyValue::Text(words(random, 3, 20)))
            }
            "Link" if random.with_probability(0.2) => Some(PropertyValue::Url(format!(
                "https://example.com/{}/{}",
                word(random),
                random.below(10_000)
            ))),
            "Points" if random.with_probability(0.5) => {
                Some(PropertyValue::Number(random.below(1_001) as f64))
            }
            _ => None,
        }
    }

    fn insert(&self, record: &Record) -> Result<(), StoreError> {
        super::kinds::insert_record(self.conn, record)
    }
}

fn positions(count: usize) -> Vec<Position> {
    Position::many_between(None, None, count).expect("open bounds always have room")
}

fn properties() -> Vec<Property> {
    let options = |names: &[&str]| -> Vec<SelectOption> {
        names
            .iter()
            .map(|name| SelectOption {
                id: SelectOptionId::generate(),
                name: (*name).into(),
            })
            .collect()
    };
    let types = [
        PropertyType::Select,
        PropertyType::Select,
        PropertyType::MultiSelect,
        PropertyType::Number,
        PropertyType::Date,
        PropertyType::Checkbox,
        PropertyType::Text,
        PropertyType::Text,
        PropertyType::Url,
        PropertyType::Number,
    ];
    PROPERTY_NAMES
        .iter()
        .zip(types)
        .map(|(name, property_type)| Property {
            id: PropertyId::generate(),
            name: (*name).into(),
            property_type,
            options: match *name {
                "Status" => options(&[
                    "Backlog",
                    "To do",
                    "In progress",
                    "In review",
                    "Done",
                    "Cancelled",
                ]),
                "Priority" => options(&["Low", "Medium", "High", "Urgent"]),
                "Labels" => options(&[
                    "bug", "feature", "ops", "design", "docs", "infra", "security", "mobile",
                    "web", "data", "billing", "support",
                ]),
                _ => Vec::new(),
            },
        })
        .collect()
}

fn views(database: &Database) -> (View, View) {
    let property = |name: &str| {
        database
            .properties
            .iter()
            .find(|property| property.name == name)
            .map(|property| property.id)
            .expect("properties() names every property this looks up")
    };
    let all_columns: Vec<PropertyId> = database
        .properties
        .iter()
        .map(|property| property.id)
        .collect();
    let all_rows = View {
        id: ViewId::generate(),
        database: database.id,
        name: "All".into(),
        view_type: ViewType::Table,
        filter: Vec::new(),
        sort: Vec::new(),
        visible_columns: all_columns.clone(),
        column_widths: Vec::new(),
    };
    let open_by_due = View {
        id: ViewId::generate(),
        name: "Open by due date".into(),
        filter: vec![FilterCondition {
            property: property("Done"),
            condition: Condition::CheckboxIs(false),
        }],
        sort: vec![
            SortKey {
                property: property("Due"),
                direction: SortDirection::Ascending,
            },
            SortKey {
                property: property("Estimate"),
                direction: SortDirection::Descending,
            },
        ],
        ..all_rows.clone()
    };
    (all_rows, open_by_due)
}

const WORDS: &[&str] = &[
    "plan",
    "review",
    "launch",
    "budget",
    "design",
    "meeting",
    "roadmap",
    "customer",
    "report",
    "draft",
    "release",
    "sprint",
    "hiring",
    "onboarding",
    "research",
    "analysis",
    "migration",
    "invoice",
    "contract",
    "feedback",
    "prototype",
    "metrics",
    "quarterly",
    "weekly",
    "notes",
    "ideas",
    "backlog",
    "support",
    "incident",
    "postmortem",
    "strategy",
    "partner",
    "pricing",
    "marketing",
    "campaign",
    "content",
    "website",
    "mobile",
    "server",
    "database",
    "search",
    "performance",
    "security",
    "audit",
    "compliance",
    "travel",
    "offsite",
    "recipe",
    "garden",
    "reading",
    "project",
    "alpha",
    "beta",
    "gamma",
    "delta",
    "north",
    "south",
    "east",
    "west",
    "spring",
    "summer",
    "autumn",
    "winter",
    "london",
    "paris",
    "berlin",
    "tokyo",
    "lisbon",
    "oslo",
    "team",
    "product",
    "engineering",
    "finance",
    "legal",
    "people",
    "sales",
    "growth",
    "retention",
    "churn",
    "pipeline",
    "forecast",
    "inventory",
    "shipping",
    "warehouse",
    "vendor",
    "training",
    "workshop",
    "interview",
    "candidate",
    "goal",
    "milestone",
    "risk",
    "decision",
    "proposal",
    "summary",
    "archive",
    "template",
    "checklist",
    "dashboard",
];

/// Low indexes are much likelier than high ones, as with popular labels or common sizes.
fn skewed_below(random: &mut Random, bound: usize) -> usize {
    let upper = random.below(bound) + 1;
    random.below(upper)
}

fn weighted(random: &mut Random, weights: &[u32]) -> usize {
    let total: u32 = weights.iter().sum();
    let mut pick = random.below(total as usize) as u32;
    for (index, weight) in weights.iter().enumerate() {
        if pick < *weight {
            return index;
        }
        pick -= weight;
    }
    weights.len() - 1
}

fn word(random: &mut Random) -> &'static str {
    random.pick(WORDS)
}

fn words(random: &mut Random, at_least: usize, at_most: usize) -> String {
    let count = at_least + random.below(at_most - at_least + 1);
    let words: Vec<&str> = (0..count).map(|_| word(random)).collect();
    words.join(" ")
}

fn date(random: &mut Random) -> Date {
    let mut date = Date {
        year: 2023 + random.below(5) as i32,
        month: 1 + random.below(12) as u8,
        day: 1,
    };
    date.day = 1 + random.below(date.days_in_month() as usize) as u8;
    date
}
