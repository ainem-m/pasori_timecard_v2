use axum::{
    Json,
    extract::State,
    http::{HeaderMap, StatusCode},
};
use pasori_core::domain::punch::{
    AttendanceDay, AttendanceDayStatus, DerivedAttendance, PolicyProfile, PunchEvent,
};
use pasori_core::domain::time::{CutoffDay, CutoffRule, MonthlyTimesheet, YearMonth};
use pasori_core::port::repo::{EmployeeRepository, PunchRepository};
use serde::Deserialize;
use uuid::Uuid;

use super::{AdminAppState, authenticate_admin_request};

#[derive(Deserialize)]
pub(super) struct MonthlyAttendanceQuery {
    employee_id: Uuid,
    year: i16,
    month: i8,
}

#[derive(serde::Serialize)]
pub(super) struct MonthlyAttendanceResponse {
    employee_id: Uuid,
    year_month: MonthlyAttendanceYearMonth,
    days: Vec<AttendanceDayResponse>,
    total_work_minutes: i64,
    derived_totals: DerivedAttendance,
    cutoff_rule: CutoffRuleResponse,
    period_start: String,
    period_end: String,
    policy_profile: PolicyProfile,
}

#[derive(serde::Serialize)]
struct MonthlyAttendanceYearMonth {
    year: i16,
    month: i8,
}

#[derive(serde::Serialize)]
struct AttendanceDayResponse {
    date: String,
    events: Vec<PunchEvent>,
    work_minutes: i64,
    derived: DerivedAttendance,
    has_inconsistency: bool,
    status: AttendanceDayStatus,
}

#[derive(serde::Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum CutoffRuleResponse {
    DayOfMonth { day: i8 },
    EndOfMonth,
}

pub(super) async fn get_monthly_attendance(
    headers: HeaderMap,
    State(state): State<AdminAppState>,
    axum::extract::Query(query): axum::extract::Query<MonthlyAttendanceQuery>,
) -> Result<Json<MonthlyAttendanceResponse>, StatusCode> {
    let _admin = authenticate_admin_request(&state, &headers).await?;

    let year_month =
        YearMonth::new(query.year, query.month).map_err(|_| StatusCode::BAD_REQUEST)?;
    let employee = state
        .repo
        .find(query.employee_id)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
        .ok_or(StatusCode::NOT_FOUND)?;
    let policy_profile = PolicyProfile::from_employment_type(&employee.employment_type);
    let cutoff_rule =
        CutoffRule::DayOfMonth(CutoffDay::new(15).map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?);
    let period = year_month
        .attendance_period(cutoff_rule)
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    let from =
        day_start_in_tokyo(period.period_start).map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    let to = day_end_in_tokyo(period.period_end).map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    let punches = state
        .repo
        .list_in_range(query.employee_id, &from, &to)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    let timesheet = build_monthly_attendance(
        query.employee_id,
        year_month,
        cutoff_rule,
        punches,
        policy_profile,
    )
    .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    Ok(Json(to_monthly_attendance_response(timesheet)))
}

fn build_monthly_attendance(
    employee_id: Uuid,
    year_month: YearMonth,
    cutoff_rule: CutoffRule,
    punches: Vec<PunchEvent>,
    policy_profile: PolicyProfile,
) -> Result<MonthlyTimesheet, pasori_core::domain::time::TimeDomainError> {
    let mut grouped: std::collections::BTreeMap<jiff::civil::Date, Vec<PunchEvent>> =
        std::collections::BTreeMap::new();

    for punch in punches {
        grouped
            .entry(punch.occurred_at.date())
            .or_default()
            .push(punch);
    }

    let days = grouped
        .into_iter()
        .map(|(date, events)| {
            AttendanceDay::from_events_with_policy_profile(
                date,
                events,
                AttendanceDayStatus::Confirmed,
                policy_profile,
            )
        })
        .collect();

    MonthlyTimesheet::from_days_with_policy_profile(
        employee_id,
        year_month,
        cutoff_rule,
        days,
        policy_profile,
    )
}

fn to_monthly_attendance_response(timesheet: MonthlyTimesheet) -> MonthlyAttendanceResponse {
    MonthlyAttendanceResponse {
        employee_id: timesheet.employee_id,
        year_month: MonthlyAttendanceYearMonth {
            year: timesheet.year_month.year(),
            month: timesheet.year_month.month(),
        },
        days: timesheet
            .days
            .into_iter()
            .map(to_attendance_day_response)
            .collect(),
        total_work_minutes: timesheet.total_work_minutes,
        derived_totals: timesheet.derived_totals,
        cutoff_rule: match timesheet.cutoff_rule {
            CutoffRule::DayOfMonth(day) => CutoffRuleResponse::DayOfMonth { day: day.value() },
            CutoffRule::EndOfMonth => CutoffRuleResponse::EndOfMonth,
        },
        period_start: timesheet.period_start.to_string(),
        period_end: timesheet.period_end.to_string(),
        policy_profile: timesheet.policy_profile,
    }
}

fn to_attendance_day_response(day: AttendanceDay) -> AttendanceDayResponse {
    AttendanceDayResponse {
        date: day.date.to_string(),
        events: day.events,
        work_minutes: day.work_minutes,
        derived: day.derived,
        has_inconsistency: day.has_inconsistency,
        status: day.status,
    }
}

pub(super) fn day_start_in_tokyo(date: jiff::civil::Date) -> Result<jiff::Zoned, jiff::Error> {
    format!("{date}T00:00:00+09:00[Asia/Tokyo]").parse()
}

pub(super) fn day_end_in_tokyo(date: jiff::civil::Date) -> Result<jiff::Zoned, jiff::Error> {
    format!("{date}T23:59:59+09:00[Asia/Tokyo]").parse()
}
