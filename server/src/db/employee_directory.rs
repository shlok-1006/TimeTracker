//! Employee directory repository — the record behind the onboarding form.
//!
//! Shapes follow the RUH HRMS "Employee & Teams" proposal (see
//! `migrations/0042_employee_directory.sql` for how its table names map onto the
//! ones this database already had). The security tiers in that document are the
//! reason the data is split, so this module keeps them apart: `DirectoryEntry`
//! and `EmployeeProfile` are the HR/PM tier, and bank details are a separate
//! type with separate functions so "who may read this" is a decision the caller
//! has to make explicitly rather than one that rides along inside a struct.

use chrono::{DateTime, NaiveDate, Utc};
use serde::{Deserialize, Serialize};
use sqlx::PgPool;
use uuid::Uuid;

use crate::error::AppError;

/// One row of the directory listing: the core (tier-1) facts only.
#[derive(Debug, Clone, Serialize)]
pub struct DirectoryEntry {
    pub user_id: Uuid,
    pub name: String,
    pub email: String,
    pub role: String,
    pub employee_code: Option<String>,
    pub department: Option<String>,
    pub designation: Option<String>,
    pub joined_on: Option<NaiveDate>,
    pub teams: Vec<String>,
    /// Whether an onboarding-form profile exists at all, so the UI can show
    /// "not submitted" rather than an empty tab that looks broken.
    pub has_profile: bool,
}

/// Personal details (tier 2). One per person.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct EmployeeProfile {
    pub date_of_birth: Option<NaiveDate>,
    pub gender: Option<String>,
    pub marital_status: Option<String>,
    pub blood_group: Option<String>,
    pub personal_email: Option<String>,
    pub phone: Option<String>,
    pub current_address: Option<String>,
    pub permanent_address: Option<String>,
    pub emergency_name: Option<String>,
    pub emergency_phone: Option<String>,
    pub emergency_relation: Option<String>,
    /// Form answers with no column yet — never dropped, just not promoted.
    #[serde(default)]
    pub extra: serde_json::Value,
    pub verified_at: Option<DateTime<Utc>>,
    pub verified_by: Option<Uuid>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Education {
    #[serde(default)]
    pub id: Option<Uuid>,
    pub degree: String,
    pub institute: Option<String>,
    /// Text on purpose — forms yield "2019", "2018-2022" and "Pursuing" alike.
    pub year: Option<String>,
    pub grade: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PrevEmployment {
    #[serde(default)]
    pub id: Option<Uuid>,
    pub company: String,
    pub title: Option<String>,
    pub from_date: Option<NaiveDate>,
    pub to_date: Option<NaiveDate>,
    pub notes: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct Document {
    pub id: Uuid,
    pub kind: String,
    pub file_name: Option<String>,
    pub storage_key: String,
    pub uploaded_at: DateTime<Utc>,
}

/// Everything the profile tabs render, minus the sealed tier.
#[derive(Debug, Clone, Serialize)]
pub struct ProfileBundle {
    pub entry: DirectoryEntry,
    pub profile: Option<EmployeeProfile>,
    pub education: Vec<Education>,
    pub prev_employment: Vec<PrevEmployment>,
    pub documents: Vec<Document>,
}

/// Bank details (tier 3). Deliberately its own type, returned only by
/// `get_bank`, which only the admin routes call.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct BankDetails {
    pub account_name: Option<String>,
    pub account_number: Option<String>,
    pub bank_name: Option<String>,
    pub ifsc: Option<String>,
    pub pan: Option<String>,
    pub uan: Option<String>,
}

/// The roster. `manager_id` = None for HR (everyone); `Some(pm)` restricts to
/// that PM's reports — the same scoping shape as `monthly_reports::list_for_month`.
pub async fn list_directory(
    pool: &PgPool,
    manager_id: Option<Uuid>,
) -> Result<Vec<DirectoryEntry>, AppError> {
    let rows = sqlx::query!(
        r#"SELECT u.id, u.name, u.email, u.role::text AS "role!",
                  u.employee_code, u.department, u.designation, u.joined_on,
                  COALESCE(
                      ARRAY(SELECT t.name FROM user_teams ut
                            JOIN teams t ON t.id = ut.team_id
                            WHERE ut.user_id = u.id ORDER BY t.name),
                      '{}'
                  ) AS "teams!: Vec<String>",
                  EXISTS (SELECT 1 FROM employee_profiles p WHERE p.user_id = u.id) AS "has_profile!"
           FROM users u
           WHERE ($1::uuid IS NULL
                  OR EXISTS (SELECT 1 FROM user_managers um
                             WHERE um.user_id = u.id AND um.manager_id = $1))
           ORDER BY u.name"#,
        manager_id
    )
    .fetch_all(pool)
    .await?;

    Ok(rows
        .into_iter()
        .map(|r| DirectoryEntry {
            user_id: r.id,
            name: r.name,
            email: r.email,
            role: r.role,
            employee_code: r.employee_code,
            department: r.department,
            designation: r.designation,
            joined_on: r.joined_on,
            teams: r.teams,
            has_profile: r.has_profile,
        })
        .collect())
}

/// One person's directory row, or None if there is no such user.
pub async fn get_entry(pool: &PgPool, user_id: Uuid) -> Result<Option<DirectoryEntry>, AppError> {
    let row = sqlx::query!(
        r#"SELECT u.id, u.name, u.email, u.role::text AS "role!",
                  u.employee_code, u.department, u.designation, u.joined_on,
                  COALESCE(
                      ARRAY(SELECT t.name FROM user_teams ut
                            JOIN teams t ON t.id = ut.team_id
                            WHERE ut.user_id = u.id ORDER BY t.name),
                      '{}'
                  ) AS "teams!: Vec<String>",
                  EXISTS (SELECT 1 FROM employee_profiles p WHERE p.user_id = u.id) AS "has_profile!"
           FROM users u WHERE u.id = $1"#,
        user_id
    )
    .fetch_optional(pool)
    .await?;

    Ok(row.map(|r| DirectoryEntry {
        user_id: r.id,
        name: r.name,
        email: r.email,
        role: r.role,
        employee_code: r.employee_code,
        department: r.department,
        designation: r.designation,
        joined_on: r.joined_on,
        teams: r.teams,
        has_profile: r.has_profile,
    }))
}

/// The full tier-2 bundle for one person. Bank details are NOT included — they
/// are a separate call so the sealed tier is never fetched by accident.
pub async fn get_bundle(pool: &PgPool, user_id: Uuid) -> Result<Option<ProfileBundle>, AppError> {
    let Some(entry) = get_entry(pool, user_id).await? else {
        return Ok(None);
    };

    let profile = sqlx::query!(
        r#"SELECT date_of_birth, gender, marital_status, blood_group, personal_email, phone,
                  current_address, permanent_address, emergency_name, emergency_phone,
                  emergency_relation, extra, verified_at, verified_by
           FROM employee_profiles WHERE user_id = $1"#,
        user_id
    )
    .fetch_optional(pool)
    .await?
    .map(|r| EmployeeProfile {
        date_of_birth: r.date_of_birth,
        gender: r.gender,
        marital_status: r.marital_status,
        blood_group: r.blood_group,
        personal_email: r.personal_email,
        phone: r.phone,
        current_address: r.current_address,
        permanent_address: r.permanent_address,
        emergency_name: r.emergency_name,
        emergency_phone: r.emergency_phone,
        emergency_relation: r.emergency_relation,
        extra: r.extra,
        verified_at: r.verified_at,
        verified_by: r.verified_by,
    });

    let education = sqlx::query!(
        "SELECT id, degree, institute, year, grade FROM employee_education
         WHERE user_id = $1 ORDER BY year DESC NULLS LAST, degree",
        user_id
    )
    .fetch_all(pool)
    .await?
    .into_iter()
    .map(|r| Education {
        id: Some(r.id),
        degree: r.degree,
        institute: r.institute,
        year: r.year,
        grade: r.grade,
    })
    .collect();

    let prev_employment = sqlx::query!(
        "SELECT id, company, title, from_date, to_date, notes FROM employee_prev_employment
         WHERE user_id = $1 ORDER BY from_date DESC NULLS LAST, company",
        user_id
    )
    .fetch_all(pool)
    .await?
    .into_iter()
    .map(|r| PrevEmployment {
        id: Some(r.id),
        company: r.company,
        title: r.title,
        from_date: r.from_date,
        to_date: r.to_date,
        notes: r.notes,
    })
    .collect();

    let documents = sqlx::query!(
        "SELECT id, kind, file_name, storage_key, uploaded_at FROM employee_documents
         WHERE user_id = $1 ORDER BY uploaded_at DESC",
        user_id
    )
    .fetch_all(pool)
    .await?
    .into_iter()
    .map(|r| Document {
        id: r.id,
        kind: r.kind,
        file_name: r.file_name,
        storage_key: r.storage_key,
        uploaded_at: r.uploaded_at,
    })
    .collect();

    Ok(Some(ProfileBundle {
        entry,
        profile,
        education,
        prev_employment,
        documents,
    }))
}

/// Create or replace the tier-1 employment facts on the core row.
pub async fn set_employment_facts(
    pool: &PgPool,
    user_id: Uuid,
    employee_code: Option<&str>,
    department: Option<&str>,
    designation: Option<&str>,
    joined_on: Option<NaiveDate>,
) -> Result<(), AppError> {
    sqlx::query!(
        "UPDATE users SET employee_code = $2, department = $3, designation = $4,
                          joined_on = $5, updated_at = now()
         WHERE id = $1",
        user_id,
        employee_code,
        department,
        designation,
        joined_on
    )
    .execute(pool)
    .await?;
    Ok(())
}

/// Create or replace the personal-details row.
pub async fn upsert_profile(
    pool: &PgPool,
    user_id: Uuid,
    p: &EmployeeProfile,
) -> Result<(), AppError> {
    sqlx::query!(
        r#"INSERT INTO employee_profiles
             (user_id, date_of_birth, gender, marital_status, blood_group, personal_email,
              phone, current_address, permanent_address, emergency_name, emergency_phone,
              emergency_relation, extra)
           VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13)
           ON CONFLICT (user_id) DO UPDATE SET
             date_of_birth = EXCLUDED.date_of_birth,
             gender = EXCLUDED.gender,
             marital_status = EXCLUDED.marital_status,
             blood_group = EXCLUDED.blood_group,
             personal_email = EXCLUDED.personal_email,
             phone = EXCLUDED.phone,
             current_address = EXCLUDED.current_address,
             permanent_address = EXCLUDED.permanent_address,
             emergency_name = EXCLUDED.emergency_name,
             emergency_phone = EXCLUDED.emergency_phone,
             emergency_relation = EXCLUDED.emergency_relation,
             extra = EXCLUDED.extra,
             updated_at = now()"#,
        user_id,
        p.date_of_birth,
        p.gender,
        p.marital_status,
        p.blood_group,
        p.personal_email,
        p.phone,
        p.current_address,
        p.permanent_address,
        p.emergency_name,
        p.emergency_phone,
        p.emergency_relation,
        p.extra
    )
    .execute(pool)
    .await?;
    Ok(())
}

/// Mark the form's answers as checked by HR. The proposal's flow is: the form
/// lands the data, HR verifies it, and from then on it is the single truth.
pub async fn mark_verified(pool: &PgPool, user_id: Uuid, by: Uuid) -> Result<(), AppError> {
    sqlx::query!(
        "UPDATE employee_profiles SET verified_at = now(), verified_by = $2, updated_at = now()
         WHERE user_id = $1",
        user_id,
        by
    )
    .execute(pool)
    .await?;
    Ok(())
}

/// Replace the whole education list for one person (simplest correct semantics
/// for a form re-submission: the latest answer wins as a set, not a merge).
pub async fn replace_education(
    pool: &PgPool,
    user_id: Uuid,
    rows: &[Education],
) -> Result<(), AppError> {
    let mut tx = pool.begin().await?;
    sqlx::query!("DELETE FROM employee_education WHERE user_id = $1", user_id)
        .execute(&mut *tx)
        .await?;
    for e in rows {
        sqlx::query!(
            "INSERT INTO employee_education (user_id, degree, institute, year, grade)
             VALUES ($1,$2,$3,$4,$5)",
            user_id,
            e.degree,
            e.institute,
            e.year,
            e.grade
        )
        .execute(&mut *tx)
        .await?;
    }
    tx.commit().await?;
    Ok(())
}

/// Replace the whole previous-employment list for one person.
pub async fn replace_prev_employment(
    pool: &PgPool,
    user_id: Uuid,
    rows: &[PrevEmployment],
) -> Result<(), AppError> {
    let mut tx = pool.begin().await?;
    sqlx::query!(
        "DELETE FROM employee_prev_employment WHERE user_id = $1",
        user_id
    )
    .execute(&mut *tx)
    .await?;
    for j in rows {
        sqlx::query!(
            "INSERT INTO employee_prev_employment (user_id, company, title, from_date, to_date, notes)
             VALUES ($1,$2,$3,$4,$5,$6)",
            user_id,
            j.company,
            j.title,
            j.from_date,
            j.to_date,
            j.notes
        )
        .execute(&mut *tx)
        .await?;
    }
    tx.commit().await?;
    Ok(())
}

/// Sealed tier. Only the admin routes call this.
pub async fn get_bank(pool: &PgPool, user_id: Uuid) -> Result<Option<BankDetails>, AppError> {
    let row = sqlx::query!(
        "SELECT account_name, account_number, bank_name, ifsc, pan, uan
         FROM employee_bank_details WHERE user_id = $1",
        user_id
    )
    .fetch_optional(pool)
    .await?;
    Ok(row.map(|r| BankDetails {
        account_name: r.account_name,
        account_number: r.account_number,
        bank_name: r.bank_name,
        ifsc: r.ifsc,
        pan: r.pan,
        uan: r.uan,
    }))
}

/// Sealed tier. Only the admin routes call this.
pub async fn upsert_bank(pool: &PgPool, user_id: Uuid, b: &BankDetails) -> Result<(), AppError> {
    sqlx::query!(
        r#"INSERT INTO employee_bank_details
             (user_id, account_name, account_number, bank_name, ifsc, pan, uan)
           VALUES ($1,$2,$3,$4,$5,$6,$7)
           ON CONFLICT (user_id) DO UPDATE SET
             account_name = EXCLUDED.account_name,
             account_number = EXCLUDED.account_number,
             bank_name = EXCLUDED.bank_name,
             ifsc = EXCLUDED.ifsc,
             pan = EXCLUDED.pan,
             uan = EXCLUDED.uan,
             updated_at = now()"#,
        user_id,
        b.account_name,
        b.account_number,
        b.bank_name,
        b.ifsc,
        b.pan,
        b.uan
    )
    .execute(pool)
    .await?;
    Ok(())
}
