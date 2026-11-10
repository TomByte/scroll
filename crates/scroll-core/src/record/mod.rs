//! Records: the queryable, permission-checked data outside page content, one module per kind
//! with its type and its field changes. All travel over the wire with postcard, so they avoid
//! serde features that need a self-describing format.

mod database;
mod page;
mod tag;
mod value;
mod view;

pub use database::{Database, DatabaseField, Property, PropertyType, SelectOption};
pub use page::{Page, PageField};
pub use tag::{PageTag, Tag, TagColor, TagField};
pub use value::{Date, PropertyValue, fits, replace_value, value_for};
pub use view::{
    ColumnWidth, Condition, FilterCondition, SortDirection, SortKey, View, ViewField, ViewType,
};
