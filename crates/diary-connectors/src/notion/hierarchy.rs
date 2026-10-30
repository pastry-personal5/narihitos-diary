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
