//! Canonical year/month path and same-day classification.

use super::*;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PageKind {
    ChildPage,
    Other,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PageChild {
    pub id: String,
    pub kind: PageKind,
    pub title: Option<String>,
    pub trashed: bool,
    pub accessible: bool,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct NotionPage {
    pub id: String,
    pub title: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct HierarchyPlan {
    pub year: Option<NotionPage>,
    pub month: Option<NotionPage>,
    pub year_will_be_created: bool,
    pub month_will_be_created: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum HierarchyError {
    Duplicate,
    WrongKind,
    Trashed,
    Inaccessible,
    IncompleteListing,
}

/// Classifies one fully paginated direct-child listing.
pub fn resolve_child(
    children: &[PageChild],
    title: &str,
    listing_complete: bool,
) -> Result<Option<NotionPage>, HierarchyError> {
    if !listing_complete {
        return Err(HierarchyError::IncompleteListing);
    }
    let matches: Vec<_> = children
        .iter()
        .filter(|child| child.title.as_deref() == Some(title))
        .collect();
    if matches.len() > 1 {
        return Err(HierarchyError::Duplicate);
    }
    let Some(found) = matches.first() else {
        return Ok(None);
    };
    if found.kind != PageKind::ChildPage {
        return Err(HierarchyError::WrongKind);
    }
    if found.trashed {
        return Err(HierarchyError::Trashed);
    }
    if !found.accessible {
        return Err(HierarchyError::Inaccessible);
    }
    Ok(Some(NotionPage {
        id: found.id.clone(),
        title: title.to_owned(),
    }))
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum SameDayState {
    Absent,
    Canonical { index: usize },
    Conflict { indices: Vec<usize> },
}

/// Finds canonical, ISO-only, and wrong-weekday headings for the target day.
pub fn classify_same_day(blocks: &[NotionBlock], plan: &NotionPlan) -> SameDayState {
    let day = &plan.heading[..10];
    let mut canonical = Vec::new();
    let mut collision = Vec::new();
    for (index, block) in blocks.iter().enumerate() {
        if block.kind != BlockKind::Heading2 {
            continue;
        }
        let text = block.display_text();
        if text == plan.heading {
            canonical.push(index);
        } else if text == *day
            || (text.starts_with(day)
                && text.as_bytes().get(10) == Some(&b' ')
                && text.as_bytes().get(11) == Some(&b'('))
        {
            collision.push(index);
        }
    }
    if canonical.len() == 1 && collision.is_empty() {
        SameDayState::Canonical {
            index: canonical[0],
        }
    } else if canonical.is_empty() && collision.is_empty() {
        SameDayState::Absent
    } else {
        collision.extend(canonical);
        collision.sort_unstable();
        SameDayState::Conflict { indices: collision }
    }
}

/// Where a new daily section goes among a month page's top-level blocks.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum InsertionPoint {
    /// After the last block, which is also where an empty page appends.
    End,
    /// Before the page's first block.
    Start,
    /// After the top-level block at this index.
    After(usize),
}

/// An insertion point plus a content-free label for Preview.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Insertion {
    pub point: InsertionPoint,
    pub label: String,
}

/// Returns the `YYYY-MM-DD` of a canonical `YYYY-MM-DD (DDD)` heading.
fn dated_heading(text: &str) -> Option<&str> {
    let bytes = text.as_bytes();
    let shape_ok = bytes.len() == 16
        && bytes[..10].iter().enumerate().all(|(index, byte)| {
            if index == 4 || index == 7 {
                *byte == b'-'
            } else {
                byte.is_ascii_digit()
            }
        })
        && &bytes[10..12] == b" ("
        && bytes[12..15].iter().all(u8::is_ascii_uppercase)
        && bytes[15] == b')';
    shape_ok.then(|| &text[..10])
}

/// Chooses where the plan's section goes so the month page stays in
/// ascending date order. Sections already out of order are left alone; the
/// new section follows the greatest earlier date, or precedes the smallest
/// later one, so it never makes the page less sorted. A section ends at the
/// next first- or second-level heading, matching read-back.
pub fn plan_insertion(blocks: &[NotionBlock], plan: &NotionPlan) -> Insertion {
    let day = &plan.heading[..10];
    let mut sections: Vec<(String, usize, usize)> = Vec::new();
    for (start, block) in blocks.iter().enumerate() {
        if block.kind != BlockKind::Heading2 {
            continue;
        }
        let Some(date) = dated_heading(&block.display_text()).map(str::to_owned) else {
            continue;
        };
        let end = blocks
            .iter()
            .enumerate()
            .skip(start + 1)
            .find(|(_, next)| matches!(next.kind, BlockKind::Heading1 | BlockKind::Heading2))
            .map_or(blocks.len(), |(index, _)| index);
        sections.push((date, start, end));
    }
    if let Some((date, _, end)) = sections
        .iter()
        .filter(|(date, ..)| date.as_str() < day)
        .max_by(|left, right| (&left.0, left.1).cmp(&(&right.0, right.1)))
    {
        let point = if *end >= blocks.len() {
            InsertionPoint::End
        } else {
            InsertionPoint::After(end - 1)
        };
        return Insertion {
            point,
            label: format!("after {date}"),
        };
    }
    if let Some((date, start, _)) = sections
        .iter()
        .filter(|(date, ..)| date.as_str() > day)
        .min_by(|left, right| (&left.0, left.1).cmp(&(&right.0, right.1)))
    {
        let point = match start.checked_sub(1) {
            Some(previous) => InsertionPoint::After(previous),
            None => InsertionPoint::Start,
        };
        return Insertion {
            point,
            label: format!("before {date}"),
        };
    }
    Insertion {
        point: InsertionPoint::End,
        label: "end".to_owned(),
    }
}
