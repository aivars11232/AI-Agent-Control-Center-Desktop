use jiff::{
    civil::DateTime,
    tz::{AmbiguousOffset, TimeZone},
    Span, Timestamp,
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{fmt, str::FromStr};

pub const MAX_SCHEDULED_ITEMS: usize = 10_000;
pub const MAX_REMINDER_TITLE_BYTES: usize = 4 * 1024;
pub const MAX_REMINDER_NOTES_BYTES: usize = 32 * 1024;
pub const MAX_RECURRENCE_OCCURRENCES: i64 = 10_000;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReminderValidationError {
    pub code: &'static str,
    pub message: String,
}

impl ReminderValidationError {
    fn new(code: &'static str, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
        }
    }
}

impl fmt::Display for ReminderValidationError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{}: {}", self.code, self.message)
    }
}

impl std::error::Error for ReminderValidationError {}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ScheduledItemKind {
    Reminder,
    Event,
}

impl ScheduledItemKind {
    pub(crate) fn as_storage_value(self) -> &'static str {
        match self {
            Self::Reminder => "reminder",
            Self::Event => "event",
        }
    }

    pub(crate) fn from_storage_value(value: &str) -> Result<Self, ReminderValidationError> {
        match value {
            "reminder" => Ok(Self::Reminder),
            "event" => Ok(Self::Event),
            _ => Err(ReminderValidationError::new(
                "REMINDER_STORAGE_INVALID",
                "The stored scheduled-item kind is invalid.",
            )),
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ScheduleStatus {
    Scheduled,
    Due,
    Completed,
    Dismissed,
    NeedsAttention,
}

impl ScheduleStatus {
    pub(crate) fn as_storage_value(self) -> &'static str {
        match self {
            Self::Scheduled => "scheduled",
            Self::Due => "due",
            Self::Completed => "completed",
            Self::Dismissed => "dismissed",
            Self::NeedsAttention => "needs_attention",
        }
    }

    pub(crate) fn from_storage_value(value: &str) -> Result<Self, ReminderValidationError> {
        match value {
            "scheduled" => Ok(Self::Scheduled),
            "due" => Ok(Self::Due),
            "completed" => Ok(Self::Completed),
            "dismissed" => Ok(Self::Dismissed),
            "needs_attention" => Ok(Self::NeedsAttention),
            _ => Err(ReminderValidationError::new(
                "REMINDER_STORAGE_INVALID",
                "The stored scheduled-item status is invalid.",
            )),
        }
    }

    #[cfg(test)]
    pub(crate) fn is_active(self) -> bool {
        matches!(self, Self::Scheduled | Self::Due | Self::NeedsAttention)
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum RecurrenceKind {
    None,
    Daily,
    Weekly,
    Monthly,
}

impl RecurrenceKind {
    pub(crate) fn as_storage_value(self) -> &'static str {
        match self {
            Self::None => "none",
            Self::Daily => "daily",
            Self::Weekly => "weekly",
            Self::Monthly => "monthly",
        }
    }

    pub(crate) fn from_storage_value(value: &str) -> Result<Self, ReminderValidationError> {
        match value {
            "none" => Ok(Self::None),
            "daily" => Ok(Self::Daily),
            "weekly" => Ok(Self::Weekly),
            "monthly" => Ok(Self::Monthly),
            _ => Err(ReminderValidationError::new(
                "REMINDER_STORAGE_INVALID",
                "The stored recurrence kind is invalid.",
            )),
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum DeliveryMode {
    InApp,
    Portal,
}

impl DeliveryMode {
    pub(crate) fn as_storage_value(self) -> &'static str {
        match self {
            Self::InApp => "in_app",
            Self::Portal => "portal",
        }
    }

    pub(crate) fn from_storage_value(value: &str) -> Result<Self, ReminderValidationError> {
        match value {
            "in_app" => Ok(Self::InApp),
            "portal" => Ok(Self::Portal),
            _ => Err(ReminderValidationError::new(
                "REMINDER_STORAGE_INVALID",
                "The stored delivery mode is invalid.",
            )),
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum PrivacyMode {
    Generic,
    Title,
}

impl PrivacyMode {
    pub(crate) fn as_storage_value(self) -> &'static str {
        match self {
            Self::Generic => "generic",
            Self::Title => "title",
        }
    }

    pub(crate) fn from_storage_value(value: &str) -> Result<Self, ReminderValidationError> {
        match value {
            "generic" => Ok(Self::Generic),
            "title" => Ok(Self::Title),
            _ => Err(ReminderValidationError::new(
                "REMINDER_STORAGE_INVALID",
                "The stored notification privacy mode is invalid.",
            )),
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum DstResolution {
    Exact,
    FoldEarlier,
    GapShiftedForward,
    Unresolved,
}

impl DstResolution {
    pub(crate) fn as_storage_value(self) -> &'static str {
        match self {
            Self::Exact => "exact",
            Self::FoldEarlier => "fold_earlier",
            Self::GapShiftedForward => "gap_shifted_forward",
            Self::Unresolved => "unresolved",
        }
    }

    pub(crate) fn from_storage_value(value: &str) -> Result<Self, ReminderValidationError> {
        match value {
            "exact" => Ok(Self::Exact),
            "fold_earlier" => Ok(Self::FoldEarlier),
            "gap_shifted_forward" => Ok(Self::GapShiftedForward),
            "unresolved" => Ok(Self::Unresolved),
            _ => Err(ReminderValidationError::new(
                "REMINDER_STORAGE_INVALID",
                "The stored DST resolution is invalid.",
            )),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RecurrenceRuleV1 {
    pub kind: RecurrenceKind,
    pub interval: i64,
    pub occurrence_limit: Option<i64>,
    pub until_unix_ms: Option<i64>,
}

impl Default for RecurrenceRuleV1 {
    fn default() -> Self {
        Self {
            kind: RecurrenceKind::None,
            interval: 1,
            occurrence_limit: None,
            until_unix_ms: None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ScheduledItemV1 {
    pub id: i64,
    pub position: i64,
    pub revision: i64,
    pub kind: ScheduledItemKind,
    pub title: String,
    pub notes: String,
    pub local_due_at: String,
    pub time_zone: String,
    pub due_at: String,
    pub due_at_unix_ms: Option<i64>,
    pub event_end_local: Option<String>,
    pub event_end_unix_ms: Option<i64>,
    pub dst_resolution: DstResolution,
    pub status: ScheduleStatus,
    pub recurrence: RecurrenceRuleV1,
    pub next_occurrence_sequence: i64,
    pub missed_occurrence_count: i64,
    pub delivery_mode: DeliveryMode,
    pub privacy_mode: PrivacyMode,
    pub schedule_fingerprint: Option<String>,
    pub subject_agent_id: Option<i64>,
    pub workspace_id: Option<String>,
    pub task_owner_agent_id: Option<i64>,
    pub task_id: Option<i64>,
    pub scheduler_agent_id: Option<i64>,
    pub schedule_issue_code: Option<String>,
    pub schedule_issue_message: Option<String>,
    pub created_at: String,
    pub created_at_unix_ms: i64,
    pub resolved_at_unix_ms: Option<i64>,
    pub updated_at_unix_ms: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ReminderOccurrenceV1 {
    pub id: i64,
    pub reminder_id: i64,
    pub schedule_revision: i64,
    pub occurrence_sequence: i64,
    pub occurrence_key: String,
    pub due_at_unix_ms: i64,
    pub status: String,
    pub missed_count: i64,
    pub first_missed_at_unix_ms: Option<i64>,
    pub last_missed_at_unix_ms: Option<i64>,
    pub detail_code: Option<String>,
    pub detail_message: Option<String>,
    pub created_at_unix_ms: i64,
    pub updated_at_unix_ms: i64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ReminderDeliveryJob {
    pub occurrence_id: i64,
    pub notification_id: String,
    pub title: String,
    pub body: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ReminderSchedulerSnapshot {
    pub revision: i64,
    pub application_state_revision: i64,
    pub system_time_zone: Option<String>,
    pub items: Vec<ScheduledItemV1>,
    pub recent_occurrences: Vec<ReminderOccurrenceV1>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CreateScheduledItemRequest {
    pub expected_revision: i64,
    pub request_id: String,
    pub kind: ScheduledItemKind,
    pub title: String,
    pub notes: String,
    pub local_due_at: String,
    pub time_zone: String,
    pub event_end_local: Option<String>,
    pub recurrence: RecurrenceRuleV1,
    pub delivery_mode: DeliveryMode,
    pub privacy_mode: PrivacyMode,
    pub subject_agent_id: Option<i64>,
    pub workspace_id: Option<String>,
    pub task_owner_agent_id: Option<i64>,
    pub task_id: Option<i64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct UpdateScheduledItemRequest {
    pub expected_revision: i64,
    pub expected_item_revision: i64,
    pub request_id: String,
    pub item_id: i64,
    pub title: String,
    pub notes: String,
    pub local_due_at: String,
    pub time_zone: String,
    pub event_end_local: Option<String>,
    pub recurrence: RecurrenceRuleV1,
    pub delivery_mode: DeliveryMode,
    pub privacy_mode: PrivacyMode,
    pub subject_agent_id: Option<i64>,
    pub workspace_id: Option<String>,
    pub task_owner_agent_id: Option<i64>,
    pub task_id: Option<i64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SetScheduledItemStatusRequest {
    pub expected_revision: i64,
    pub expected_item_revision: i64,
    pub request_id: String,
    pub item_id: i64,
    pub status: ScheduleStatus,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct DeleteScheduledItemRequest {
    pub expected_revision: i64,
    pub expected_item_revision: i64,
    pub request_id: String,
    pub item_id: i64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScheduleResolution {
    pub local_due_at: String,
    pub time_zone: String,
    pub due_at: String,
    pub due_at_unix_ms: i64,
    pub dst_resolution: DstResolution,
}

#[cfg(test)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DueWindowClassification {
    Overdue,
    DueNow,
    DueWithinWindow,
    Future,
    Inactive,
    NeedsAttention,
}

pub fn system_time_zone_name() -> Option<String> {
    TimeZone::try_system()
        .ok()
        .and_then(|zone| zone.iana_name().map(str::to_owned))
}

pub fn resolve_local_due_at(
    local_due_at: &str,
    time_zone: &str,
) -> Result<ScheduleResolution, ReminderValidationError> {
    require_bounded_text(
        "localDueAt",
        local_due_at,
        128,
        false,
        "REMINDER_LOCAL_TIME_INVALID",
    )?;
    require_bounded_text(
        "timeZone",
        time_zone,
        256,
        false,
        "REMINDER_TIME_ZONE_INVALID",
    )?;

    let local = DateTime::from_str(local_due_at).map_err(|_| {
        ReminderValidationError::new(
            "REMINDER_LOCAL_TIME_INVALID",
            "localDueAt must be an ISO local date and time without a UTC suffix.",
        )
    })?;
    let zone = TimeZone::get(time_zone).map_err(|_| {
        ReminderValidationError::new(
            "REMINDER_TIME_ZONE_INVALID",
            format!("`{time_zone}` is not an available IANA time-zone identifier."),
        )
    })?;
    let ambiguous = zone.to_ambiguous_zoned(local);
    let dst_resolution = match ambiguous.offset() {
        AmbiguousOffset::Unambiguous { .. } => DstResolution::Exact,
        AmbiguousOffset::Gap { .. } => DstResolution::GapShiftedForward,
        AmbiguousOffset::Fold { .. } => DstResolution::FoldEarlier,
    };
    let zoned = ambiguous.compatible().map_err(|_| {
        ReminderValidationError::new(
            "REMINDER_LOCAL_TIME_UNRESOLVED",
            "The local date and time cannot be resolved in the selected time zone.",
        )
    })?;
    let timestamp = zoned.timestamp();
    let due_at_unix_ms = timestamp.as_millisecond();
    if due_at_unix_ms < 0 {
        return Err(ReminderValidationError::new(
            "REMINDER_LOCAL_TIME_INVALID",
            "Scheduled dates before the Unix epoch are not supported.",
        ));
    }

    Ok(ScheduleResolution {
        local_due_at: local.to_string(),
        time_zone: time_zone.to_string(),
        due_at: timestamp.to_string(),
        due_at_unix_ms,
        dst_resolution,
    })
}

pub fn validate_recurrence(recurrence: &RecurrenceRuleV1) -> Result<(), ReminderValidationError> {
    if !(1..=366).contains(&recurrence.interval) {
        return Err(ReminderValidationError::new(
            "REMINDER_RECURRENCE_INVALID",
            "A recurrence interval must be between 1 and 366.",
        ));
    }
    if let Some(limit) = recurrence.occurrence_limit {
        if !(1..=MAX_RECURRENCE_OCCURRENCES).contains(&limit) {
            return Err(ReminderValidationError::new(
                "REMINDER_RECURRENCE_INVALID",
                "An occurrence limit must be between 1 and 10000.",
            ));
        }
    }
    if recurrence.until_unix_ms.is_some_and(|value| value < 0) {
        return Err(ReminderValidationError::new(
            "REMINDER_RECURRENCE_INVALID",
            "A recurrence end timestamp must be non-negative.",
        ));
    }
    if recurrence.kind == RecurrenceKind::None
        && (recurrence.interval != 1
            || recurrence.occurrence_limit.is_some()
            || recurrence.until_unix_ms.is_some())
    {
        return Err(ReminderValidationError::new(
            "REMINDER_RECURRENCE_INVALID",
            "A non-recurring item cannot include recurrence limits.",
        ));
    }
    Ok(())
}

pub fn recurrence_resolution(
    anchor_local_due_at: &str,
    time_zone: &str,
    recurrence: &RecurrenceRuleV1,
    sequence: i64,
) -> Result<Option<ScheduleResolution>, ReminderValidationError> {
    validate_recurrence(recurrence)?;
    if !(0..=MAX_RECURRENCE_OCCURRENCES).contains(&sequence) {
        return Err(ReminderValidationError::new(
            "REMINDER_RECURRENCE_INVALID",
            "The occurrence sequence is outside the supported range.",
        ));
    }
    if recurrence.kind == RecurrenceKind::None {
        return (sequence == 0)
            .then(|| resolve_local_due_at(anchor_local_due_at, time_zone))
            .transpose();
    }
    if recurrence
        .occurrence_limit
        .is_some_and(|limit| sequence >= limit)
    {
        return Ok(None);
    }

    let anchor = DateTime::from_str(anchor_local_due_at).map_err(|_| {
        ReminderValidationError::new(
            "REMINDER_LOCAL_TIME_INVALID",
            "The recurrence anchor is not a valid ISO local date and time.",
        )
    })?;
    let amount = sequence.checked_mul(recurrence.interval).ok_or_else(|| {
        ReminderValidationError::new(
            "REMINDER_RECURRENCE_INVALID",
            "The recurrence sequence is too large.",
        )
    })?;
    let span = match recurrence.kind {
        RecurrenceKind::None => Ok(Span::new()),
        RecurrenceKind::Daily => Span::new().try_days(amount),
        RecurrenceKind::Weekly => Span::new().try_weeks(amount),
        RecurrenceKind::Monthly => Span::new().try_months(amount),
    }
    .map_err(|_| {
        ReminderValidationError::new(
            "REMINDER_RECURRENCE_INVALID",
            "The recurrence is outside the supported calendar range.",
        )
    })?;
    let next_local = anchor.checked_add(span).map_err(|_| {
        ReminderValidationError::new(
            "REMINDER_RECURRENCE_INVALID",
            "The recurrence resolves outside the supported calendar range.",
        )
    })?;
    let resolution = resolve_local_due_at(&next_local.to_string(), time_zone)?;
    if recurrence
        .until_unix_ms
        .is_some_and(|until| resolution.due_at_unix_ms > until)
    {
        return Ok(None);
    }
    Ok(Some(resolution))
}

pub fn validate_create_request(
    request: &CreateScheduledItemRequest,
) -> Result<ScheduleResolution, ReminderValidationError> {
    validate_request_common(
        request.expected_revision,
        &request.request_id,
        request.kind,
        &request.title,
        &request.notes,
        &request.local_due_at,
        &request.time_zone,
        request.event_end_local.as_deref(),
        &request.recurrence,
        request.workspace_id.as_deref(),
        request.task_owner_agent_id,
        request.task_id,
    )
}

pub fn validate_update_request(
    request: &UpdateScheduledItemRequest,
    kind: ScheduledItemKind,
) -> Result<ScheduleResolution, ReminderValidationError> {
    if request.item_id <= 0 || request.expected_item_revision <= 0 {
        return Err(ReminderValidationError::new(
            "REMINDER_REQUEST_INVALID",
            "The item identifier and expected item revision must be positive.",
        ));
    }
    validate_request_common(
        request.expected_revision,
        &request.request_id,
        kind,
        &request.title,
        &request.notes,
        &request.local_due_at,
        &request.time_zone,
        request.event_end_local.as_deref(),
        &request.recurrence,
        request.workspace_id.as_deref(),
        request.task_owner_agent_id,
        request.task_id,
    )
}

#[allow(clippy::too_many_arguments)]
fn validate_request_common(
    expected_revision: i64,
    request_id: &str,
    kind: ScheduledItemKind,
    title: &str,
    notes: &str,
    local_due_at: &str,
    time_zone: &str,
    event_end_local: Option<&str>,
    recurrence: &RecurrenceRuleV1,
    workspace_id: Option<&str>,
    task_owner_agent_id: Option<i64>,
    task_id: Option<i64>,
) -> Result<ScheduleResolution, ReminderValidationError> {
    if expected_revision < 0 {
        return Err(ReminderValidationError::new(
            "REMINDER_REQUEST_INVALID",
            "The expected scheduler revision must be non-negative.",
        ));
    }
    require_bounded_text(
        "requestId",
        request_id,
        128,
        false,
        "REMINDER_REQUEST_INVALID",
    )?;
    require_bounded_text(
        "title",
        title,
        MAX_REMINDER_TITLE_BYTES,
        false,
        "REMINDER_REQUEST_INVALID",
    )?;
    require_bounded_text(
        "notes",
        notes,
        MAX_REMINDER_NOTES_BYTES,
        true,
        "REMINDER_REQUEST_INVALID",
    )?;
    if let Some(workspace_id) = workspace_id {
        require_bounded_text(
            "workspaceId",
            workspace_id,
            4 * 1024,
            false,
            "REMINDER_REQUEST_INVALID",
        )?;
    }
    if task_owner_agent_id.is_some() != task_id.is_some()
        || task_owner_agent_id.is_some_and(|value| value <= 0)
        || task_id.is_some_and(|value| value <= 0)
    {
        return Err(ReminderValidationError::new(
            "REMINDER_REQUEST_INVALID",
            "A task link requires positive owner and task identifiers together.",
        ));
    }
    validate_recurrence(recurrence)?;
    let resolution = resolve_local_due_at(local_due_at, time_zone)?;

    match (kind, event_end_local) {
        (ScheduledItemKind::Reminder, Some(_)) => {
            return Err(ReminderValidationError::new(
                "REMINDER_REQUEST_INVALID",
                "A reminder cannot include an event end time.",
            ));
        }
        (ScheduledItemKind::Event, Some(end)) => {
            let end = resolve_local_due_at(end, time_zone)?;
            if end.due_at_unix_ms <= resolution.due_at_unix_ms {
                return Err(ReminderValidationError::new(
                    "REMINDER_REQUEST_INVALID",
                    "An event end time must be later than its start time.",
                ));
            }
        }
        _ => {}
    }

    Ok(resolution)
}

pub fn portal_policy_fingerprint(schedule_fingerprint: &str) -> String {
    format!(
        "{:x}",
        Sha256::digest(format!("reminder-portal-policy-v1|{schedule_fingerprint}"))
    )
}

pub fn schedule_fingerprint(
    request: &CreateScheduledItemRequest,
    resolution: &ScheduleResolution,
) -> Result<String, ReminderValidationError> {
    let canonical = serde_json::to_vec(&(
        "reminder-schedule-v1",
        request.kind,
        request.title.trim(),
        request.notes.as_str(),
        &resolution.local_due_at,
        &resolution.time_zone,
        resolution.due_at_unix_ms,
        request.event_end_local.as_deref(),
        &request.recurrence,
        request.delivery_mode,
        request.privacy_mode,
        request.subject_agent_id,
        request.workspace_id.as_deref(),
        request.task_owner_agent_id,
        request.task_id,
    ))
    .map_err(|_| {
        ReminderValidationError::new(
            "REMINDER_REQUEST_INVALID",
            "The scheduled item could not be canonicalized.",
        )
    })?;
    Ok(format!("{:x}", Sha256::digest(canonical)))
}

pub fn validate_scheduled_item(item: &ScheduledItemV1) -> Result<(), ReminderValidationError> {
    const MAX_SAFE_IDENTIFIER: i64 = 9_007_199_254_740_991;
    if !(1..=MAX_SAFE_IDENTIFIER).contains(&item.id)
        || !(0..=MAX_SAFE_IDENTIFIER).contains(&item.position)
        || !(1..=MAX_SAFE_IDENTIFIER).contains(&item.revision)
        || !(0..=MAX_RECURRENCE_OCCURRENCES).contains(&item.next_occurrence_sequence)
        || item.missed_occurrence_count < 0
        || item.created_at_unix_ms < 0
        || item.updated_at_unix_ms < item.created_at_unix_ms
        || item.resolved_at_unix_ms.is_some_and(|value| value < 0)
        || item
            .subject_agent_id
            .is_some_and(|value| !(1..=MAX_SAFE_IDENTIFIER).contains(&value))
        || item
            .scheduler_agent_id
            .is_some_and(|value| !(1..=MAX_SAFE_IDENTIFIER).contains(&value))
        || item.task_owner_agent_id.is_some() != item.task_id.is_some()
        || item
            .task_owner_agent_id
            .is_some_and(|value| !(1..=MAX_SAFE_IDENTIFIER).contains(&value))
        || item
            .task_id
            .is_some_and(|value| !(1..=MAX_SAFE_IDENTIFIER).contains(&value))
    {
        return Err(ReminderValidationError::new(
            "REMINDER_STORAGE_INVALID",
            "The stored scheduled-item identifiers, revisions, or timestamps are invalid.",
        ));
    }
    require_bounded_text(
        "title",
        &item.title,
        MAX_REMINDER_TITLE_BYTES,
        false,
        "REMINDER_STORAGE_INVALID",
    )?;
    require_bounded_text(
        "notes",
        &item.notes,
        2 * 1024 * 1024,
        true,
        "REMINDER_STORAGE_INVALID",
    )?;
    require_bounded_text(
        "createdAt",
        &item.created_at,
        128,
        false,
        "REMINDER_STORAGE_INVALID",
    )?;
    require_bounded_text(
        "localDueAt",
        &item.local_due_at,
        128,
        false,
        "REMINDER_STORAGE_INVALID",
    )?;
    require_bounded_text(
        "timeZone",
        &item.time_zone,
        256,
        false,
        "REMINDER_STORAGE_INVALID",
    )?;
    require_bounded_text(
        "dueAt",
        &item.due_at,
        128,
        false,
        "REMINDER_STORAGE_INVALID",
    )?;
    if item
        .workspace_id
        .as_ref()
        .is_some_and(|value| value.trim().is_empty() || value.len() > 4 * 1024)
        || item
            .schedule_issue_code
            .as_ref()
            .is_some_and(|value| value.trim().is_empty() || value.len() > 4 * 1024)
        || item
            .schedule_issue_message
            .as_ref()
            .is_some_and(|value| value.trim().is_empty() || value.len() > 32 * 1024)
    {
        return Err(ReminderValidationError::new(
            "REMINDER_STORAGE_INVALID",
            "The stored scheduled-item links or issue evidence are invalid.",
        ));
    }
    validate_recurrence(&item.recurrence)?;
    if item.schedule_fingerprint.as_ref().is_some_and(|value| {
        value.len() != 64 || !value.bytes().all(|byte| byte.is_ascii_hexdigit())
    }) || (item.delivery_mode == DeliveryMode::Portal && item.schedule_fingerprint.is_none())
        || (matches!(
            item.status,
            ScheduleStatus::Completed | ScheduleStatus::Dismissed
        ) && item.resolved_at_unix_ms.is_none())
        || (item.status == ScheduleStatus::NeedsAttention)
            != (item.schedule_issue_code.is_some() && item.schedule_issue_message.is_some())
    {
        return Err(ReminderValidationError::new(
            "REMINDER_STORAGE_INVALID",
            "The stored notification authority or terminal evidence is incomplete.",
        ));
    }
    if item.due_at_unix_ms.is_none() {
        let unresolved_end_is_consistent = matches!(
            (
                item.kind,
                item.event_end_local.as_deref(),
                item.event_end_unix_ms,
            ),
            (ScheduledItemKind::Reminder, None, None)
                | (ScheduledItemKind::Event, None, None)
                | (ScheduledItemKind::Event, Some(_), Some(0..))
        );
        if item.status != ScheduleStatus::NeedsAttention
            || item.dst_resolution != DstResolution::Unresolved
            || item.schedule_issue_code.is_none()
            || item.schedule_issue_message.is_none()
            || !unresolved_end_is_consistent
        {
            return Err(ReminderValidationError::new(
                "REMINDER_STORAGE_INVALID",
                "An unresolved scheduled item requires consistent fields and inspectable issue evidence.",
            ));
        }
        return Ok(());
    }
    let anchor = resolve_local_due_at(&item.local_due_at, &item.time_zone)?;
    let due_at_unix_ms = match item.due_at_unix_ms {
        Some(value) if value >= 0 => {
            let parsed = Timestamp::from_str(&item.due_at).map_err(|_| {
                ReminderValidationError::new(
                    "REMINDER_STORAGE_INVALID",
                    "The stored due instant is not a valid timestamp.",
                )
            })?;
            if parsed.as_millisecond() != value || item.dst_resolution == DstResolution::Unresolved
            {
                return Err(ReminderValidationError::new(
                    "REMINDER_STORAGE_INVALID",
                    "The stored due instant and DST evidence do not agree.",
                ));
            }
            value
        }
        _ => {
            return Err(ReminderValidationError::new(
                "REMINDER_STORAGE_INVALID",
                "A scheduled item requires a resolved due instant unless it needs attention.",
            ));
        }
    };

    let expected_current = recurrence_resolution(
        &item.local_due_at,
        &item.time_zone,
        &item.recurrence,
        item.next_occurrence_sequence,
    );
    let expected_previous = (item.next_occurrence_sequence > 0)
        .then(|| {
            recurrence_resolution(
                &item.local_due_at,
                &item.time_zone,
                &item.recurrence,
                item.next_occurrence_sequence - 1,
            )
        })
        .transpose()?
        .flatten();
    let expected_matches = |resolution: &ScheduleResolution| {
        resolution.due_at_unix_ms == due_at_unix_ms
            && resolution.dst_resolution == item.dst_resolution
    };
    let current_matches = match expected_current {
        Ok(resolution) => resolution.as_ref().is_some_and(expected_matches),
        Err(error)
            if item.status == ScheduleStatus::NeedsAttention
                && item.schedule_issue_code.as_deref() == Some(error.code) =>
        {
            false
        }
        Err(error) => return Err(error),
    };
    if !current_matches && !expected_previous.as_ref().is_some_and(expected_matches) {
        return Err(ReminderValidationError::new(
            "REMINDER_STORAGE_INVALID",
            "The stored due instant does not match its recurrence anchor and sequence.",
        ));
    }

    match (
        item.kind,
        item.event_end_local.as_deref(),
        item.event_end_unix_ms,
    ) {
        (ScheduledItemKind::Reminder, None, None) => {}
        (ScheduledItemKind::Event, None, None) => {}
        (ScheduledItemKind::Event, Some(local_end), Some(end_unix_ms)) => {
            let end = resolve_local_due_at(local_end, &item.time_zone)?;
            if end.due_at_unix_ms != end_unix_ms || end_unix_ms <= anchor.due_at_unix_ms {
                return Err(ReminderValidationError::new(
                    "REMINDER_STORAGE_INVALID",
                    "The stored event end does not match its local time or follows no start.",
                ));
            }
        }
        _ => {
            return Err(ReminderValidationError::new(
                "REMINDER_STORAGE_INVALID",
                "Reminder/event end fields are inconsistent.",
            ));
        }
    }
    Ok(())
}

#[cfg(test)]
pub fn classify_due_window(
    item: &ScheduledItemV1,
    now_unix_ms: i64,
    due_window_end_unix_ms: i64,
) -> DueWindowClassification {
    if !item.status.is_active() {
        return DueWindowClassification::Inactive;
    }
    let Some(due_at) = item.due_at_unix_ms else {
        return DueWindowClassification::NeedsAttention;
    };
    if due_at < now_unix_ms {
        DueWindowClassification::Overdue
    } else if due_at == now_unix_ms {
        DueWindowClassification::DueNow
    } else if due_at <= due_window_end_unix_ms {
        DueWindowClassification::DueWithinWindow
    } else {
        DueWindowClassification::Future
    }
}

fn require_bounded_text(
    field: &str,
    value: &str,
    max_bytes: usize,
    allow_empty: bool,
    code: &'static str,
) -> Result<(), ReminderValidationError> {
    if (!allow_empty && value.trim().is_empty()) || value.len() > max_bytes {
        return Err(ReminderValidationError::new(
            code,
            format!("{field} must be non-empty when required and no more than {max_bytes} bytes."),
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn item(due_at_unix_ms: Option<i64>) -> ScheduledItemV1 {
        ScheduledItemV1 {
            id: 1,
            position: 0,
            revision: 1,
            kind: ScheduledItemKind::Reminder,
            title: "Test".to_string(),
            notes: String::new(),
            local_due_at: "2026-08-28T12:00:00".to_string(),
            time_zone: "UTC".to_string(),
            due_at: "2026-08-28T12:00:00Z".to_string(),
            due_at_unix_ms,
            event_end_local: None,
            event_end_unix_ms: None,
            dst_resolution: DstResolution::Exact,
            status: ScheduleStatus::Scheduled,
            recurrence: RecurrenceRuleV1::default(),
            next_occurrence_sequence: 0,
            missed_occurrence_count: 0,
            delivery_mode: DeliveryMode::InApp,
            privacy_mode: PrivacyMode::Generic,
            schedule_fingerprint: None,
            subject_agent_id: None,
            workspace_id: None,
            task_owner_agent_id: None,
            task_id: None,
            scheduler_agent_id: None,
            schedule_issue_code: None,
            schedule_issue_message: None,
            created_at: "2026-08-28T10:00:00Z".to_string(),
            created_at_unix_ms: 1,
            resolved_at_unix_ms: None,
            updated_at_unix_ms: 1,
        }
    }

    #[test]
    fn task_0018_reminder_resolves_local_time_and_dst_deterministically() {
        let ordinary = resolve_local_due_at("2026-08-28T12:30:00", "Europe/Amsterdam").unwrap();
        assert_eq!(ordinary.dst_resolution, DstResolution::Exact);
        assert_eq!(ordinary.due_at, "2026-08-28T10:30:00Z");

        let gap = resolve_local_due_at("2026-03-08T02:30:00", "America/New_York").unwrap();
        assert_eq!(gap.dst_resolution, DstResolution::GapShiftedForward);
        assert_eq!(gap.due_at, "2026-03-08T07:30:00Z");

        let fold = resolve_local_due_at("2026-11-01T01:30:00", "America/New_York").unwrap();
        assert_eq!(fold.dst_resolution, DstResolution::FoldEarlier);
        assert_eq!(fold.due_at, "2026-11-01T05:30:00Z");
    }

    #[test]
    fn task_0018_reminder_due_window_never_counts_overdue_as_upcoming() {
        assert_eq!(
            classify_due_window(&item(Some(99)), 100, 200),
            DueWindowClassification::Overdue
        );
        assert_eq!(
            classify_due_window(&item(Some(100)), 100, 200),
            DueWindowClassification::DueNow
        );
        assert_eq!(
            classify_due_window(&item(Some(150)), 100, 200),
            DueWindowClassification::DueWithinWindow
        );
    }

    #[test]
    fn task_0018_reminder_monthly_recurrence_uses_the_original_anchor() {
        let recurrence = RecurrenceRuleV1 {
            kind: RecurrenceKind::Monthly,
            interval: 1,
            occurrence_limit: None,
            until_unix_ms: None,
        };
        let february =
            recurrence_resolution("2027-01-31T09:00:00", "Europe/Amsterdam", &recurrence, 1)
                .unwrap()
                .unwrap();
        let march =
            recurrence_resolution("2027-01-31T09:00:00", "Europe/Amsterdam", &recurrence, 2)
                .unwrap()
                .unwrap();
        assert_eq!(february.local_due_at, "2027-02-28T09:00:00");
        assert_eq!(march.local_due_at, "2027-03-31T09:00:00");
    }

    #[test]
    fn task_0018_reminder_portal_authority_is_backend_derived() {
        let request = CreateScheduledItemRequest {
            expected_revision: 0,
            request_id: "request-1".to_string(),
            kind: ScheduledItemKind::Reminder,
            title: "Test".to_string(),
            notes: String::new(),
            local_due_at: "2026-08-28T12:30:00".to_string(),
            time_zone: "Europe/Amsterdam".to_string(),
            event_end_local: None,
            recurrence: RecurrenceRuleV1::default(),
            delivery_mode: DeliveryMode::Portal,
            privacy_mode: PrivacyMode::Generic,
            subject_agent_id: None,
            workspace_id: None,
            task_owner_agent_id: None,
            task_id: None,
        };
        let resolution = validate_create_request(&request).unwrap();
        let schedule = schedule_fingerprint(&request, &resolution).unwrap();
        let policy = portal_policy_fingerprint(&schedule);
        assert_eq!(policy.len(), 64);
        assert_ne!(policy, schedule);
    }
}
