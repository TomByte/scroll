//! The showcase workspace the demo records: an invented travel-app team's pages, a roadmap
//! database, diagrams, code and tags. Written through the store's normal local write path, as
//! the MCP server writes, so the app opens it like any workspace.

use std::collections::HashMap;
use std::ops::Range;
use std::path::Path;

use scroll_core::id::{DatabaseId, PageId, PropertyId, SelectOptionId, TagId, ViewId};
use scroll_core::operation::{Operation, Record, Transaction};
use scroll_core::page_doc::{BlockKind, BlockPlacement, Mark, PageDoc};
use scroll_core::position::Position;
use scroll_core::record::{
    ColumnWidth, Condition, Database, Date, FilterCondition, Page, PageTag, Property, PropertyType,
    PropertyValue, SelectOption, SortDirection, SortKey, Tag, TagColor, View, ViewType,
};
use scroll_core::store::WorkspaceStore;

type SeedResult<T> = Result<T, Box<dyn std::error::Error>>;

/// One block: kind, nesting depth and text in a small inline markup: `**bold**`, `*italic*`,
/// `` `code` ``, `~~struck~~` and `[label](url)`. A to-do starting `[x] ` is checked; a callout
/// starts with its icon and a space.
type SeedBlock = (BlockKind, usize, &'static str);

use BlockKind::{
    BulletedListItem as Bullet, Callout, Heading2 as H2, Heading3 as H3,
    NumberedListItem as Numbered, Paragraph, Quote, ToDo,
};

/// The pages the window starts with: the one shown, and the ones left expanded in the sidebar.
pub struct Showcase {
    pub home: PageId,
    pub expanded: Vec<PageId>,
}

const HOME: &[SeedBlock] = &[
    (
        Paragraph,
        0,
        "Welcome to **Fernway**, the trip planner that works offline. This page is the front door: what we are building, how we work, and where things live.",
    ),
    (
        Callout,
        0,
        "🎯 **This quarter:** ship offline maps and shared itineraries by *December 12*.",
    ),
    (H2, 0, "This week"),
    (ToDo, 0, "[x] Freeze the `2.4` release branch"),
    (ToDo, 0, "[x] Review the ~~sync v1~~ sync v2 design"),
    (ToDo, 0, "Write the release notes for `2.4.0`"),
    (ToDo, 0, "Book the December offsite"),
    (H2, 0, "How we work"),
    (
        Bullet,
        0,
        "Plans live in the **Roadmap** database; specs hang off each row.",
    ),
    (
        Bullet,
        0,
        "Decisions get a page under **Engineering** or **Design**.",
    ),
    (Bullet, 1, "Short and dated, with the *why* written down."),
    (Bullet, 0, "Everything is one `⌘P` away."),
    (H3, 0, "Shipping a change"),
    (Numbered, 0, "Open a draft and link it from its roadmap row"),
    (Numbered, 0, "Get one review from outside the area"),
    (
        Numbered,
        0,
        "Merge, then post a short note in the release channel",
    ),
    (
        Quote,
        0,
        "Make the common case fast and the rare case possible.",
    ),
    (
        Paragraph,
        0,
        "New here? Start with the [onboarding guide](https://example.com/onboarding), then read **Sync architecture**.",
    ),
];

const SYNC_FLOW: &str = "flowchart LR
    A[Edit] --> B[(Local file)]
    B --> C{Online?}
    C -->|Yes| D[Server]
    C -->|No| B
    D --> E[Teammates]";

const APPLY_CHANGE: &str =
    "/// Applies a change on this device at once, then queues it for the server.
pub fn apply(&mut self, change: Change) -> Result<Receipt, SyncError> {
    let receipt = self.store.write(&change)?;
    self.outbox.push(change);
    if self.is_online() {
        self.flush_soon();
    }
    Ok(receipt)
}";

const SYNC_SEQUENCE: &str = "sequenceDiagram
    participant App
    participant Server
    participant Peer
    App->>Server: push(transaction)
    Server-->>App: ack(seq 42)
    Server->>Peer: broadcast(seq 42)";

const RELEASE_CHECKLIST: &[SeedBlock] = &[
    (
        Paragraph,
        0,
        "Run through this before every release. Copy it into the release's own page.",
    ),
    (H2, 0, "Before the freeze"),
    (
        ToDo,
        0,
        "[x] Every roadmap row for the release is **Shipped** or moved",
    ),
    (ToDo, 0, "[x] Crash-free sessions above 99.5% on the beta"),
    (ToDo, 0, "Translations merged"),
    (H2, 0, "Release day"),
    (ToDo, 0, "Tag the build and attach the notes"),
    (ToDo, 0, "Staged rollout: 10%, then 50%, then everyone"),
    (
        Callout,
        0,
        "⚠️ If anything looks off, pause the rollout first and ask questions second.",
    ),
];

const API_STYLE: &[SeedBlock] = &[
    (
        Paragraph,
        0,
        "Resources are plural nouns; actions are verbs on them. Times are UTC, in ISO 8601.",
    ),
    (Bullet, 0, "`GET /trips/:id/stops` lists a trip's stops"),
    (Bullet, 0, "`POST /trips/:id/share` invites someone"),
    (
        Bullet,
        0,
        "Errors carry a stable `code` and a human `message`",
    ),
];

const DESIGN_PRINCIPLES: &[SeedBlock] = &[
    (H2, 0, "Calm"),
    (
        Paragraph,
        0,
        "Travel is stressful enough. Quiet colours, generous space, one primary action per screen.",
    ),
    (H2, 0, "Offline first"),
    (
        Paragraph,
        0,
        "Every screen works on a plane. Network states are a footnote, not a modal.",
    ),
    (H2, 0, "Fast"),
    (
        Paragraph,
        0,
        "If it takes longer than a tap, it shows progress.",
    ),
];

const COMPONENTS: &[SeedBlock] = &[
    (Bullet, 0, "Itinerary card"),
    (Bullet, 0, "Map pin and cluster"),
    (Bullet, 0, "Booking summary"),
    (Bullet, 1, "Flight, hotel and car variants"),
];

const WEEKLY_NOV_10: &[SeedBlock] = &[
    (H3, 0, "Updates"),
    (Bullet, 0, "Offline map packs are live for everyone"),
    (Bullet, 0, "Shared itineraries in internal beta"),
    (H3, 0, "Actions"),
    (ToDo, 0, "Draft the 2.4 announcement"),
    (ToDo, 0, "[x] Pick a venue for the offsite"),
];

const WEEKLY_NOV_3: &[SeedBlock] = &[
    (H3, 0, "Updates"),
    (Bullet, 0, "Search across trips shipped"),
    (Bullet, 0, "Accessibility audit started"),
];

const RETRO: &[SeedBlock] = &[
    (H3, 0, "Went well"),
    (Bullet, 0, "Cold start is twice as fast"),
    (H3, 0, "To change"),
    (Bullet, 0, "Fewer late scope changes"),
];

const READING: &[SeedBlock] = &[
    (
        ToDo,
        0,
        "[x] [Local-first software](https://www.inkandswitch.com/local-first/)",
    ),
    (
        ToDo,
        0,
        "[Designing Data-Intensive Applications](https://dataintensive.net)",
    ),
    (ToDo, 0, "*The Design of Everyday Things*"),
];

/// Builds the workspace in a new file at `path` and returns the pages the window starts on.
pub fn build(path: &Path) -> SeedResult<Showcase> {
    let mut seeder = Seeder {
        store: WorkspaceStore::open(path)?,
        last_positions: HashMap::new(),
    };
    let tags = seeder.tags()?;

    let home = seeder.document(None, "🏠", "Team home", HOME)?;
    seeder.tag(home, tags.handbook)?;
    let roadmap = seeder.page(None, Some("🗺️"), "Roadmap")?;
    seeder.tag(roadmap, tags.planning)?;
    seeder.roadmap(roadmap)?;

    let engineering = seeder.document(
        None,
        "⚙️",
        "Engineering",
        &[(Paragraph, 0, "How Fernway is built, and why.")],
    )?;
    let sync = seeder.page(Some(engineering), Some("🔄"), "Sync architecture")?;
    seeder.sync_architecture(sync)?;
    seeder.tag(sync, tags.engineering)?;
    let checklist = seeder.document(
        Some(engineering),
        "🚀",
        "Release checklist",
        RELEASE_CHECKLIST,
    )?;
    seeder.tag(checklist, tags.engineering)?;
    let api = seeder.document(Some(engineering), "🧭", "API style guide", API_STYLE)?;
    seeder.tag(api, tags.draft)?;

    let design = seeder.document(
        None,
        "🎨",
        "Design",
        &[(Paragraph, 0, "Principles, components and reviews.")],
    )?;
    let principles = seeder.document(Some(design), "✏️", "Design principles", DESIGN_PRINCIPLES)?;
    seeder.tag(principles, tags.design)?;
    seeder.document(Some(design), "🧩", "Component inventory", COMPONENTS)?;

    let meetings = seeder.document(
        None,
        "🗓️",
        "Meetings",
        &[(Paragraph, 0, "Notes from every team meeting, newest first.")],
    )?;
    seeder.document(Some(meetings), "📝", "Weekly sync, Nov 10", WEEKLY_NOV_10)?;
    seeder.document(Some(meetings), "📝", "Weekly sync, Nov 3", WEEKLY_NOV_3)?;
    seeder.document(Some(meetings), "🔁", "October retro", RETRO)?;
    seeder.document(None, "📚", "Reading list", READING)?;

    Ok(Showcase {
        home,
        expanded: vec![engineering, design],
    })
}

struct Tags {
    handbook: TagId,
    planning: TagId,
    engineering: TagId,
    design: TagId,
    draft: TagId,
}

struct Seeder {
    store: WorkspaceStore,
    /// The last position given out under each parent, so pages keep the order they are made in.
    last_positions: HashMap<Option<PageId>, Position>,
}

impl Seeder {
    fn apply(&mut self, operations: Vec<Operation>) -> SeedResult<()> {
        self.store
            .apply_local_transaction(&Transaction::new(operations))?;
        Ok(())
    }

    fn tags(&mut self) -> SeedResult<Tags> {
        let mut tag = |name: &str, color: TagColor| -> SeedResult<TagId> {
            let tag = Tag {
                id: TagId::generate(),
                name: name.to_string(),
                color,
            };
            let id = tag.id;
            self.apply(vec![Operation::Create(Record::Tag(tag))])?;
            Ok(id)
        };
        Ok(Tags {
            handbook: tag("handbook", TagColor::Blue)?,
            planning: tag("planning", TagColor::Green)?,
            engineering: tag("engineering", TagColor::Purple)?,
            design: tag("design", TagColor::Pink)?,
            draft: tag("draft", TagColor::Yellow)?,
        })
    }

    fn tag(&mut self, page: PageId, tag: TagId) -> SeedResult<()> {
        self.apply(vec![Operation::Create(Record::PageTag(PageTag {
            page,
            tag,
        }))])
    }

    fn page(
        &mut self,
        parent: Option<PageId>,
        icon: Option<&str>,
        title: &str,
    ) -> SeedResult<PageId> {
        let page = self.new_page(parent, icon, title)?;
        let id = page.id;
        self.apply(vec![Operation::Create(Record::Page(page))])?;
        Ok(id)
    }

    fn new_page(
        &mut self,
        parent: Option<PageId>,
        icon: Option<&str>,
        title: &str,
    ) -> SeedResult<Page> {
        let position = Position::between(self.last_positions.get(&parent), None)?;
        self.last_positions.insert(parent, position.clone());
        Ok(Page {
            id: PageId::generate(),
            parent,
            title: title.to_string(),
            icon: icon.map(str::to_string),
            position,
            row_of: None,
            is_archived: false,
        })
    }

    fn document(
        &mut self,
        parent: Option<PageId>,
        icon: &str,
        title: &str,
        blocks: &[SeedBlock],
    ) -> SeedResult<PageId> {
        let page = self.page(parent, Some(icon), title)?;
        self.write_content(page, |doc| write_blocks(doc, blocks))?;
        Ok(page)
    }

    /// Stores what `write` puts in a new document as the page's content, as the editor would.
    fn write_content(
        &mut self,
        page: PageId,
        write: impl FnOnce(&PageDoc) -> SeedResult<()>,
    ) -> SeedResult<()> {
        let mut doc = PageDoc::new();
        write(&doc)?;
        if let Some(update) = doc.take_local_update()? {
            self.store.record_local_page_update(page, &update)?;
        }
        self.store.compact_page(page)?;
        Ok(())
    }

    fn sync_architecture(&mut self, page: PageId) -> SeedResult<()> {
        self.write_content(page, |doc| {
            write_blocks(
                doc,
                &[
                    (
                        Paragraph,
                        0,
                        "How an edit travels from one device to everyone else's. Page text is a CRDT, so concurrent typing merges; records are operations the server puts in order.",
                    ),
                    (H2, 0, "The path of an edit"),
                ],
            )?;
            code_block(doc, "mermaid", SYNC_FLOW)?;
            write_blocks(doc, &[(H2, 0, "Applying a change")])?;
            code_block(doc, "rust", APPLY_CHANGE)?;
            write_blocks(doc, &[(H2, 0, "Acknowledgement")])?;
            code_block(doc, "mermaid", SYNC_SEQUENCE)?;
            write_blocks(
                doc,
                &[(
                    Paragraph,
                    0,
                    "The server's sequence number decides which write wins, field by field.",
                )],
            )
        })
    }

    fn roadmap(&mut self, host: PageId) -> SeedResult<()> {
        let schema = RoadmapSchema::new();
        let database = Database {
            id: DatabaseId::generate(),
            host_page: host,
            properties: schema.properties(),
        };
        let database_id = database.id;
        let views = schema.views(database_id);
        let mut operations = vec![Operation::Create(Record::Database(database))];
        operations.extend(
            views
                .into_iter()
                .map(|view| Operation::Create(Record::View(view))),
        );
        self.apply(operations)?;
        for row in ROADMAP_ROWS {
            let mut page = self.new_page(Some(host), None, row.title)?;
            page.row_of = Some(database_id);
            let id = page.id;
            let mut operations = vec![Operation::Create(Record::Page(page))];
            operations.extend(
                schema
                    .values(row)
                    .into_iter()
                    .map(|(property, value)| Operation::set_value(id, property, Some(value))),
            );
            self.apply(operations)?;
        }
        Ok(())
    }
}

fn write_blocks(doc: &PageDoc, blocks: &[SeedBlock]) -> SeedResult<()> {
    let mut parents = Vec::new();
    for &(kind, depth, source) in blocks {
        parents.truncate(depth);
        let parent = parents.last().copied();
        let block = doc.insert_block(BlockPlacement::LastChildOf(parent), kind)?;
        let (source, is_checked) = match source.strip_prefix("[x] ") {
            Some(rest) => (rest, true),
            None => (source, false),
        };
        let source = match (kind, source.split_once(' ')) {
            (Callout, Some((icon, rest))) => {
                doc.set_block_icon(block, Some(icon))?;
                rest
            }
            _ => source,
        };
        let (text, marks) = parse_inline(source);
        doc.insert_text(block, 0, &text)?;
        for (range, mark) in marks {
            doc.apply_mark(block, range, &mark)?;
        }
        if is_checked {
            doc.toggle_to_do(block)?;
        }
        parents.push(block);
    }
    Ok(())
}

fn code_block(doc: &PageDoc, language: &str, code: &str) -> SeedResult<()> {
    let block = doc.insert_block(BlockPlacement::LastChildOf(None), BlockKind::Code)?;
    doc.insert_text(block, 0, code)?;
    doc.set_block_language(block, Some(language))?;
    Ok(())
}

/// Splits the seed markup into plain text and the marks over it (byte ranges).
fn parse_inline(source: &str) -> (String, Vec<(Range<usize>, Mark)>) {
    let mut text = String::new();
    let mut marks = Vec::new();
    let mut rest = source;
    while !rest.is_empty() {
        if let Some((inner, mark, after)) = marked_span(rest) {
            let start = text.len();
            text.push_str(inner);
            marks.push((start..text.len(), mark));
            rest = after;
            continue;
        }
        let mut characters = rest.chars();
        if let Some(character) = characters.next() {
            text.push(character);
        }
        rest = characters.as_str();
    }
    (text, marks)
}

/// The marked span `rest` starts with, if any: its text, its mark and what follows it.
fn marked_span(rest: &str) -> Option<(&str, Mark, &str)> {
    if let Some(after_bracket) = rest.strip_prefix('[') {
        let (label, after_label) = after_bracket.split_once("](")?;
        let (url, after) = after_label.split_once(')')?;
        return Some((label, Mark::Link(url.to_string()), after));
    }
    let delimiters = [
        ("**", Mark::Bold),
        ("~~", Mark::Strikethrough),
        ("`", Mark::InlineCode),
        ("*", Mark::Italic),
    ];
    let (delimiter, mark) = delimiters
        .into_iter()
        .find(|(delimiter, _)| rest.starts_with(delimiter))?;
    let (inner, after) = rest[delimiter.len()..].split_once(delimiter)?;
    Some((inner, mark, after))
}

struct RoadmapRow {
    title: &'static str,
    status: &'static str,
    areas: &'static [&'static str],
    owner: &'static str,
    due: (i32, u8, u8),
    estimate: f64,
}

/// Options take the swatch of their place in the list, so these orders give each its colour:
/// grey, brown, orange, yellow, green, blue, purple, pink.
const STATUSES: &[&str] = &[
    "Backlog",
    "Blocked",
    "In progress",
    "In review",
    "Shipped",
    "Planned",
];
const AREAS: &[&str] = &[
    "Infra", "Billing", "Platform", "Growth", "Maps", "Sync", "Editor", "Design",
];

const fn row(
    title: &'static str,
    status: &'static str,
    areas: &'static [&'static str],
    owner: &'static str,
    due: (i32, u8, u8),
    estimate: f64,
) -> RoadmapRow {
    RoadmapRow {
        title,
        status,
        areas,
        owner,
        due,
        estimate,
    }
}

/// In due-date order; the tour counts rows down to "Live trip notes".
const ROADMAP_ROWS: &[RoadmapRow] = &[
    row(
        "Offline map packs",
        "Shipped",
        &["Maps"],
        "Kenji",
        (2026, 10, 14),
        8.0,
    ),
    row(
        "Faster cold start",
        "Shipped",
        &["Infra"],
        "Eli",
        (2026, 10, 21),
        5.0,
    ),
    row(
        "Dark mode polish",
        "Shipped",
        &["Design"],
        "Lena",
        (2026, 10, 30),
        3.0,
    ),
    row(
        "Search across trips",
        "Shipped",
        &["Editor"],
        "Ava",
        (2026, 11, 4),
        5.0,
    ),
    row(
        "Accessibility audit",
        "In review",
        &["Design"],
        "Lena",
        (2026, 11, 19),
        3.0,
    ),
    row(
        "Shared itineraries",
        "In progress",
        &["Sync", "Editor"],
        "Ava",
        (2026, 11, 21),
        13.0,
    ),
    row(
        "Trip budget tracker",
        "In progress",
        &["Editor"],
        "Ben",
        (2026, 11, 24),
        5.0,
    ),
    row(
        "Live trip notes",
        "Planned",
        &["Sync"],
        "Dev",
        (2026, 11, 28),
        8.0,
    ),
    row(
        "Flight status widget",
        "Planned",
        &["Platform"],
        "Marco",
        (2026, 12, 5),
        5.0,
    ),
    row(
        "Calendar sync",
        "Blocked",
        &["Sync"],
        "Dev",
        (2026, 12, 10),
        8.0,
    ),
    row(
        "Packing list templates",
        "Planned",
        &["Editor"],
        "Nia",
        (2026, 12, 12),
        3.0,
    ),
    row(
        "Weather on itinerary",
        "Planned",
        &["Maps"],
        "Marco",
        (2026, 12, 16),
        2.0,
    ),
    row(
        "Hotel import from email",
        "Backlog",
        &["Growth"],
        "Farah",
        (2026, 12, 19),
        8.0,
    ),
    row(
        "Currency converter",
        "Backlog",
        &["Billing"],
        "Chloé",
        (2027, 1, 16),
        2.0,
    ),
    row(
        "Map annotations",
        "Backlog",
        &["Maps", "Design"],
        "Kenji",
        (2027, 1, 23),
        5.0,
    ),
    row(
        "Photo journal",
        "Backlog",
        &["Editor", "Design"],
        "Chloé",
        (2027, 2, 6),
        13.0,
    ),
];

/// The roadmap's properties, made once so rows and views can refer to them by id.
struct RoadmapSchema {
    status: Property,
    due: Property,
    areas: Property,
    owner: Property,
    is_public: Property,
    estimate: Property,
    spec: Property,
}

impl RoadmapSchema {
    /// Created in display order: the store keeps properties ordered by id, which is creation order.
    fn new() -> RoadmapSchema {
        RoadmapSchema {
            status: property("Status", PropertyType::Select, STATUSES),
            due: property("Due", PropertyType::Date, &[]),
            areas: property("Area", PropertyType::MultiSelect, AREAS),
            owner: property("Owner", PropertyType::Text, &[]),
            is_public: property("Public", PropertyType::Checkbox, &[]),
            estimate: property("Days", PropertyType::Number, &[]),
            spec: property("Spec", PropertyType::Url, &[]),
        }
    }

    fn properties(&self) -> Vec<Property> {
        [
            &self.status,
            &self.due,
            &self.areas,
            &self.owner,
            &self.is_public,
            &self.estimate,
            &self.spec,
        ]
        .into_iter()
        .cloned()
        .collect()
    }

    /// "By due date" first, so the table opens sorted; "In progress" shows a filter. The widths
    /// fit every column but the spec link beside the sidebar.
    fn views(&self, database: DatabaseId) -> Vec<View> {
        let columns: Vec<PropertyId> = self
            .properties()
            .iter()
            .map(|property| property.id)
            .collect();
        let widths = [
            (&self.status, 112),
            (&self.due, 112),
            (&self.areas, 140),
            (&self.owner, 84),
            (&self.is_public, 70),
            (&self.estimate, 64),
        ];
        let by_due = View {
            id: ViewId::generate(),
            database,
            name: "By due date".into(),
            view_type: ViewType::Table,
            filter: Vec::new(),
            sort: vec![SortKey {
                property: self.due.id,
                direction: SortDirection::Ascending,
            }],
            visible_columns: columns,
            column_widths: widths
                .into_iter()
                .map(|(property, width)| ColumnWidth {
                    property: property.id,
                    width,
                })
                .collect(),
        };
        let in_progress = View {
            id: ViewId::generate(),
            name: "In progress".into(),
            sort: Vec::new(),
            filter: vec![FilterCondition {
                property: self.status.id,
                condition: Condition::SelectIs(option(&self.status, "In progress")),
            }],
            ..by_due.clone()
        };
        vec![by_due, in_progress]
    }

    fn values(&self, row: &RoadmapRow) -> Vec<(PropertyId, PropertyValue)> {
        let (year, month, day) = row.due;
        let areas = row
            .areas
            .iter()
            .map(|area| option(&self.areas, area))
            .collect();
        let spec = format!("https://example.com/specs/{}", slug(row.title));
        vec![
            (
                self.status.id,
                PropertyValue::Select(option(&self.status, row.status)),
            ),
            (self.due.id, PropertyValue::Date(Date { year, month, day })),
            (self.areas.id, PropertyValue::MultiSelect(areas)),
            (self.owner.id, PropertyValue::Text(row.owner.to_string())),
            (
                self.is_public.id,
                PropertyValue::Checkbox(row.status == "Shipped"),
            ),
            (self.estimate.id, PropertyValue::Number(row.estimate)),
            (self.spec.id, PropertyValue::Url(spec)),
        ]
    }
}

/// "Offline map packs" becomes "offline-map-packs".
fn slug(title: &str) -> String {
    title.to_lowercase().replace(' ', "-")
}

fn property(name: &str, property_type: PropertyType, options: &[&str]) -> Property {
    Property {
        id: PropertyId::generate(),
        name: name.to_string(),
        property_type,
        options: options
            .iter()
            .map(|name| SelectOption {
                id: SelectOptionId::generate(),
                name: (*name).to_string(),
            })
            .collect(),
    }
}

/// The id of the option called `name`; the rows above only name options the schema has.
fn option(property: &Property, name: &str) -> SelectOptionId {
    property
        .options
        .iter()
        .find(|option| option.name == name)
        .map(|option| option.id)
        .unwrap_or_else(|| panic!("{} has no option {name}", property.name))
}
