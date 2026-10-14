//! Keyset (cursor) pagination for list endpoints (SOP API-006, DB-007).
//!
//! Offsets are banned: `OFFSET n` scans and discards `n` rows and skips or
//! repeats rows under concurrent inserts. Lists page by the last seen `UUIDv7`
//! id (time-ordered), carried to the client as an opaque cursor.
//!
//! ```text
//! SELECT ... WHERE ($after::uuid IS NULL OR id < $after) ORDER BY id DESC LIMIT $fetch_limit
//! ```

use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use serde::{Deserialize, Serialize};
use utoipa::{IntoParams, ToSchema};
use uuid::Uuid;

use crate::ApiError;

pub const DEFAULT_LIMIT: u32 = 50;
pub const MAX_LIMIT: u32 = 100;

/// Query parameters accepted by every list endpoint.
#[derive(Debug, Clone, Default, Deserialize, IntoParams)]
#[serde(deny_unknown_fields)]
#[into_params(parameter_in = Query)]
pub struct PageParams {
    /// Page size, 1..=100. Defaults to 50.
    pub limit: Option<u32>,
    /// Opaque cursor from a previous page's `next_cursor`.
    pub cursor: Option<String>,
}

/// Validated pagination request.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PageRequest {
    limit: u32,
    after: Option<Uuid>,
}

impl PageRequest {
    /// First page with the default size.
    pub const fn first() -> Self {
        Self {
            limit: DEFAULT_LIMIT,
            after: None,
        }
    }

    pub const fn limit(&self) -> u32 {
        self.limit
    }

    /// Exclusive lower bound (in descending id order) for the next page.
    pub const fn after(&self) -> Option<Uuid> {
        self.after
    }

    /// Rows to fetch: one more than the page size reveals whether a next page exists.
    pub fn fetch_limit(&self) -> i64 {
        i64::from(self.limit) + 1
    }

    /// Splits an over-fetched row set into a page and its next cursor.
    pub fn paginate<T>(&self, mut rows: Vec<T>, id_of: impl Fn(&T) -> Uuid) -> Page<T> {
        let page_size = usize::try_from(self.limit).unwrap_or(usize::MAX);
        let has_more = rows.len() > page_size;
        rows.truncate(page_size);
        let next_cursor = if has_more {
            rows.last().map(|row| encode_cursor(id_of(row)))
        } else {
            None
        };
        Page {
            items: rows,
            next_cursor,
        }
    }
}

impl TryFrom<PageParams> for PageRequest {
    type Error = ApiError;

    fn try_from(params: PageParams) -> Result<Self, Self::Error> {
        let limit = params.limit.unwrap_or(DEFAULT_LIMIT);
        if !(1..=MAX_LIMIT).contains(&limit) {
            return Err(ApiError::validation(
                "limit",
                format!("must be between 1 and {MAX_LIMIT}"),
            ));
        }
        let after = params.cursor.as_deref().map(decode_cursor).transpose()?;
        Ok(Self { limit, after })
    }
}

/// One page of results.
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct Page<T> {
    pub items: Vec<T>,
    /// Pass as `cursor` to fetch the next page. Absent on the last page.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub next_cursor: Option<String>,
}

impl<T> Page<T> {
    /// Converts every item, keeping the cursor.
    pub fn map<U>(self, f: impl FnMut(T) -> U) -> Page<U> {
        Page {
            items: self.items.into_iter().map(f).collect(),
            next_cursor: self.next_cursor,
        }
    }
}

fn encode_cursor(id: Uuid) -> String {
    URL_SAFE_NO_PAD.encode(id.as_bytes())
}

fn decode_cursor(cursor: &str) -> Result<Uuid, ApiError> {
    let invalid = || ApiError::validation("cursor", "is not a valid cursor");
    let bytes = URL_SAFE_NO_PAD.decode(cursor).map_err(|_| invalid())?;
    Uuid::from_slice(&bytes).map_err(|_| invalid())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ids(n: u128) -> Vec<Uuid> {
        (1..=n).map(Uuid::from_u128).collect()
    }

    #[test]
    fn cursor_round_trips() {
        let id = Uuid::from_u128(42);
        assert_eq!(decode_cursor(&encode_cursor(id)).unwrap(), id);
    }

    #[test]
    fn rejects_out_of_range_limit_and_garbage_cursor() {
        let too_big = PageParams {
            limit: Some(MAX_LIMIT + 1),
            cursor: None,
        };
        let garbage = PageParams {
            limit: None,
            cursor: Some("!!".to_owned()),
        };
        assert!(PageRequest::try_from(too_big).is_err());
        assert!(PageRequest::try_from(garbage).is_err());
    }

    #[test]
    fn paginate_emits_cursor_only_when_more_rows_exist() {
        let request = PageRequest::try_from(PageParams {
            limit: Some(2),
            cursor: None,
        })
        .unwrap();

        let full = request.paginate(ids(3), |id| *id);
        let last = request.paginate(ids(2), |id| *id);

        assert_eq!(full.items.len(), 2);
        assert_eq!(full.next_cursor, Some(encode_cursor(Uuid::from_u128(2))));
        assert_eq!(last.next_cursor, None);
    }
}
