/*!
 * A rust library for interacting with the Airtable API.
 *
 * For more information, the Airtable API is documented at [airtable.com/api](https://airtable.com/api).
 *
 * Example:
 *
 * ```ignore
 * use airtable_api::{Airtable, Record};
 * use serde::{Deserialize, Serialize};
 *
 * async fn get_records() {
 *     // Initialize the Airtable client.
 *     let airtable = Airtable::new_without_enterprise_id("API_KEY", "BASE_ID");
 *
 *     // Get the current records from a table.
 *     let mut records: Vec<Record<SomeFormat>> = airtable
 *         .list_records(
 *             "Table Name",
 *             "Grid view",
 *             vec!["the", "fields", "you", "want", "to", "return"],
 *         )
 *         .await
 *         .unwrap();
 *
 *     // Iterate over the records.
 *     for (i, record) in records.clone().iter().enumerate() {
 *         println!("{} - {:?}", i, record);
 *     }
 * }
 *
 * #[derive(Debug, Clone, Serialize, Deserialize)]
 * pub struct SomeFormat {
 *     pub x: bool,
 * }
 * ```
 */
#![allow(clippy::field_reassign_with_default)]
use std::{env, fmt, fmt::Debug, marker::PhantomData};

use chrono::{offset::Utc, DateTime};
use reqwest::{header, Method, Request, StatusCode, Url};
use schemars::JsonSchema;
use serde::{
    de::{DeserializeOwned, MapAccess, SeqAccess, Visitor},
    Deserialize, Deserializer, Serialize,
};
use thiserror::Error;

pub mod formulas;
mod list_builder;
mod macros;
pub mod markdown;

pub use list_builder::ListRecords;
pub use macros::*;

/// Errors returned by the Airtable client.
#[derive(Debug, Error)]
pub enum Error {
    #[error(transparent)]
    UrlParse(#[from] url::ParseError),
    #[error(transparent)]
    InvalidHeaderValue(#[from] reqwest::header::InvalidHeaderValue),
    #[error(transparent)]
    Reqwest(#[from] reqwest::Error),
    #[error(transparent)]
    ReqwestMiddleware(#[from] reqwest_middleware::Error),
    #[error("status code: {status}, body: {body}")]
    ApiError { status: StatusCode, body: String },
    #[error("enterprise account id is required")]
    MissingEnterpriseAccountId,
    #[error("no user was returned")]
    NoUserReturned,
    #[error("delete user failed: {0:?}")]
    DeleteUserFailed(DeleteUserResponse),
    #[error("invalid formula value: {0}")]
    InvalidFormulaValue(String),
    #[error("failed to parse record: {source:?}, record: {record:?}")]
    ParseResponseFailed {
        source: RecordParsingError,
        record: Record<serde_json::Value>,
    },
}

pub type Result<T> = std::result::Result<T, Error>;

/// Endpoint for the Airtable API.
const ENDPOINT: &str = "https://api.airtable.com/v0/";

trait _Private {}
#[expect(private_bounds)]
pub trait HasEnterpriseIdMarker: Clone + _Private {}
#[derive(Debug, Clone)]
pub struct DoesNotHaveEnterpriseId;
impl _Private for DoesNotHaveEnterpriseId {}
impl HasEnterpriseIdMarker for DoesNotHaveEnterpriseId {}
#[derive(Debug, Clone)]
pub struct HasEnterpriseId {
    enterprise_account_id: String,
}
impl _Private for HasEnterpriseId {}
impl HasEnterpriseIdMarker for HasEnterpriseId {}

/// Entrypoint for interacting with the Airtable API.
#[derive(Clone)]
pub struct Airtable<HEIM: HasEnterpriseIdMarker = DoesNotHaveEnterpriseId> {
    key: String,
    base_id: String,
    enterprise_account_id: HEIM,

    pub(crate) client: reqwest_middleware::ClientWithMiddleware,
}

/// Get the API key from the `AIRTABLE_API_KEY` env variable.
#[must_use]
pub fn api_key_from_env() -> String {
    env::var("AIRTABLE_API_KEY").unwrap_or_default()
}

impl Airtable<DoesNotHaveEnterpriseId> {
    pub fn new_without_enterprise_id<K, B>(key: K, base_id: B) -> Self
    where
        K: ToString,
        B: ToString,
    {
        Airtable::new(key, base_id, DoesNotHaveEnterpriseId)
    }
}

impl<HEIM: HasEnterpriseIdMarker> Airtable<HEIM> {
    /// Create a new Airtable client struct. It takes a type that can convert into
    /// an &str (`String` or `Vec<u8>` for example). As long as the function is
    /// given a valid API Key and Base ID your requests will work.
    /// You can leave the Enterprise Account ID empty if you are not using the
    /// Enterprise API features.
    pub fn new<K, B>(key: K, base_id: B, enterprise_account_id: HEIM) -> Self
    where
        K: ToString,
        B: ToString,
    {
        let http = reqwest::Client::builder().build();
        match http {
            Ok(c) => {
                let retry_policy = reqwest_retry::policies::ExponentialBackoff::builder()
                    .build_with_max_retries(3);
                let client = reqwest_middleware::ClientBuilder::new(c)
                    // Trace HTTP requests. See the tracing crate to make use of these traces.
                    .with(reqwest_tracing::TracingMiddleware::default())
                    // Retry failed requests.
                    .with(reqwest_retry::RetryTransientMiddleware::new_with_policy(
                        retry_policy,
                    ))
                    .build();

                Self {
                    key: key.to_string(),
                    base_id: base_id.to_string(),
                    enterprise_account_id,

                    client,
                }
            }
            Err(err) => panic!("creating client failed: {err:?}"),
        }
    }

    pub fn new_with_enterprise_id<K, B>(
        key: K,
        base_id: B,
        enterprise_account_id: String,
    ) -> Airtable<HasEnterpriseId>
    where
        K: ToString,
        B: ToString,
    {
        Airtable::new(
            key,
            base_id,
            HasEnterpriseId {
                enterprise_account_id,
            },
        )
    }

    /// The ID of the base this client talks to.
    #[must_use]
    pub fn base_id(&self) -> &str {
        &self.base_id
    }

    /// Get the currently set API key.
    #[must_use]
    pub fn get_key(&self) -> &str {
        &self.key
    }

    pub(crate) fn request<B>(
        &self,
        method: Method,
        path: String,
        body: B,
        query: Option<Vec<(&str, String)>>,
    ) -> Result<Request>
    where
        B: Serialize,
    {
        let base = Url::parse(ENDPOINT)?;
        let url = base.join(&(self.base_id.clone() + "/" + &path))?;

        let bt = format!("Bearer {}", self.key);
        let bearer = header::HeaderValue::from_str(&bt)?;

        // Set the default headers.
        let mut headers = header::HeaderMap::new();
        headers.append(header::AUTHORIZATION, bearer);
        headers.append(
            header::CONTENT_TYPE,
            header::HeaderValue::from_static("application/json"),
        );

        let mut rb = self.client.request(method.clone(), url).headers(headers);

        match query {
            None => (),
            Some(val) => {
                rb = rb.query(&val);
            }
        }

        // Add the body, this is to ensure our GET and DELETE calls succeed.
        if method != Method::GET && method != Method::DELETE {
            rb = rb.json(&body);
        }

        // Build the request.
        Ok(rb.build()?)
    }

    pub async fn list_filtered_querieable_records<
        T: macros::QueriedRecord,
        F: macros::filters::TableFilter<Table = T::Table>,
    >(
        &self,
        filter: F,
        max_records: Option<u32>,
    ) -> Result<Vec<T>> {
        let (return_fields_by_id, fields) = T::queried_fields();
        let records = self
            .list_records_build::<serde_json::Value>(&list_builder::ListRecords {
                table_name: T::Table::TABLE_NAME.to_string(),
                view_name: None,
                max_records,
                fields,
                filter_formula: Some(filter.filter_formula()),
                return_fields_by_id,
            })
            .await?;
        let mut res = Vec::with_capacity(records.len());
        for record in records {
            match T::from_record(&record) {
                Ok(r) => res.push(r),
                Err(e) => {
                    return Err(Error::ParseResponseFailed {
                        source: e,
                        record: record.clone(),
                    });
                }
            }
        }
        Ok(res)
    }

    /// Lists every record of `T`'s table (no filter) and parses each one leniently: a record
    /// that fails to parse yields an `Err` with the parsing error and the raw record instead of
    /// aborting the whole listing. `extra_fields` are fetched in addition to
    /// `T::queried_fields()` (e.g. a primary field used as a label) and must use the same
    /// naming mode (IDs or names) as `T`.
    pub async fn list_querieable_records_lenient<T: macros::QueriedRecord>(
        &self,
        extra_fields: &[&'static str],
    ) -> Result<Vec<RecordParse<T>>> {
        let (return_fields_by_id, mut fields) = T::queried_fields();
        fields.extend_from_slice(extra_fields);
        let records = self
            .list_records_build::<serde_json::Value>(&list_builder::ListRecords {
                table_name: T::Table::TABLE_NAME.to_string(),
                view_name: None,
                max_records: None,
                fields,
                filter_formula: None,
                return_fields_by_id,
            })
            .await?;
        Ok(records
            .into_iter()
            .map(|record| {
                T::from_record(&record).map_err(|error| RecordParseFailure { error, record })
            })
            .collect())
    }

    /// Fetches the schema (tables and their fields) of the base from the Meta API.
    /// Requires the `schema.bases:read` scope on the token.
    pub async fn get_base_schema(&self) -> Result<BaseSchema> {
        let url = Url::parse(ENDPOINT)?.join(&format!("meta/bases/{}/tables", self.base_id))?;
        let resp = self
            .client
            .get(url)
            .header(header::AUTHORIZATION, format!("Bearer {}", self.key))
            .send()
            .await?;
        match resp.status() {
            StatusCode::OK => (),
            s => {
                return Err(Error::ApiError {
                    status: s,
                    body: resp.text().await?,
                });
            }
        }
        Ok(resp.json().await?)
    }

    /// List records in a table for a particular view.
    pub async fn list_records_build<T: DeserializeOwned>(
        &self,
        builder: &list_builder::ListRecords,
    ) -> Result<Vec<Record<T>>> {
        let mut pages = self.pages(builder);
        let mut records = Vec::new();
        while let Some(mut page) = pages.next().await? {
            records.append(&mut page);
        }
        Ok(records)
    }

    #[must_use]
    pub fn pages<'a, T: DeserializeOwned>(
        &'a self,
        builder: &'a list_builder::ListRecords,
    ) -> Pages<'a, T, HEIM> {
        Pages::new(self, builder)
    }

    /// Get record from a table.
    pub async fn get_record<T: DeserializeOwned>(
        &self,
        builder: &list_builder::GetRecord,
    ) -> Result<Record<T>> {
        // Build the request.
        let request = self.request(
            Method::GET,
            format!("{}/{}", builder.table_name, builder.record_id),
            (),
            Some(vec![(
                "returnFieldsByFieldId",
                builder.return_fields_by_id.to_string(),
            )]),
        )?;

        let resp = self.client.execute(request).await?;
        match resp.status() {
            StatusCode::OK => (),
            s => {
                return Err(Error::ApiError {
                    status: s,
                    body: resp.text().await?,
                });
            }
        }

        // Try to deserialize the response.
        let record: Record<T> = resp.json().await?;

        Ok(record)
    }

    pub async fn get_querieable_record<T: macros::QueriedRecord>(
        &self,
        record_id: &str,
    ) -> Result<T> {
        let (return_fields_by_id, _fields) = T::queried_fields();
        let record = self
            .get_record::<serde_json::Value>(&list_builder::GetRecord {
                table_name: T::Table::TABLE_NAME.to_string(),
                record_id: record_id.to_string(),
                return_fields_by_id,
            })
            .await?;
        match T::from_record(&record) {
            Ok(r) => Ok(r),
            Err(e) => Err(Error::ParseResponseFailed {
                source: e,
                record: record.clone(),
            }),
        }
    }

    /// Delete record from a table.
    pub async fn delete_record(&self, table: &str, record_id: &str) -> Result<()> {
        // Build the request.
        let request = self.request(
            Method::DELETE,
            table.to_string(),
            (),
            Some(vec![("records[]", record_id.to_string())]),
        )?;

        let resp = self.client.execute(request).await?;
        match resp.status() {
            StatusCode::OK => (),
            s => {
                return Err(Error::ApiError {
                    status: s,
                    body: resp.text().await?,
                });
            }
        }

        Ok(())
    }

    /// Delete multiple records from a table.
    ///
    /// Due to limitations on the Airtable API, you can only bulk delete 10
    /// records at a time.
    pub async fn delete_records(
        &self,
        table: &str,
        record_ids: impl IntoIterator<Item = &str>,
    ) -> Result<()> {
        // Build the request.
        let request = self.request(
            Method::DELETE,
            table.to_string(),
            (),
            Some(
                record_ids
                    .into_iter()
                    .map(|record_id| ("records[]", record_id.to_string()))
                    .collect(),
            ),
        )?;

        let resp = self.client.execute(request).await?;
        match resp.status() {
            StatusCode::OK => (),
            s => {
                return Err(Error::ApiError {
                    status: s,
                    body: resp.text().await?,
                });
            }
        }

        Ok(())
    }

    /// Bulk create records in a table.
    ///
    /// The Airtable API limits record creation requests to 10 records per request. Because of
    /// this, calls to this function containing more than 10 records will send multiple requests
    /// (one request for each chunk of 10 records) to the Airtable API.
    pub async fn create_querieable_records<In: Serialize, Out: macros::QueriedRecord>(
        &self,
        records: Vec<Record<In>>,
    ) -> Result<Vec<Out>> {
        let table = Out::Table::TABLE_NAME;
        let (return_fields_by_id, _fields) = Out::queried_fields();

        let mut created_records = Vec::new();
        let mut records = records.into_iter();
        while records.len() > 0 {
            let chunk: Vec<_> = (&mut records).take(10).collect();
            let request = self.request(
                Method::POST,
                table.to_string(),
                APICall {
                    records: chunk,
                    offset: String::new(),
                    typecast: Some(true),
                    perform_upsert: None,
                    return_fields_by_field_id: Some(return_fields_by_id),
                },
                None,
            )?;
            let resp = self.client.execute(request).await?;
            match resp.status() {
                StatusCode::OK => (),
                s => {
                    return Err(Error::ApiError {
                        status: s,
                        body: resp.text().await?,
                    });
                }
            }
            let r: APICall<_> = resp.json().await?;
            created_records.extend(r.records);
        }

        let mut res = Vec::with_capacity(created_records.len());
        for record in created_records {
            match Out::from_record(&record) {
                Ok(r) => res.push(r),
                Err(e) => {
                    return Err(Error::ParseResponseFailed {
                        source: e,
                        record: record.clone(),
                    });
                }
            }
        }
        Ok(res)
    }

    /// Bulk update records in a table.
    ///
    /// The Airtable API limits record update requests to 10 records per request. Because of
    /// this, calls to this function containing more than 10 records will send multiple requests
    /// (one request for each chunk of 10 records) to the Airtable API.
    pub async fn update_records<T: Serialize>(
        &self,
        table: &str,
        records: Vec<Record<T>>,
    ) -> Result<()> {
        let mut records = records.into_iter();
        while records.len() > 0 {
            let chunk = (&mut records).take(10).collect();
            self.update_records_inner::<_, serde::de::IgnoredAny>(table, chunk, None)
                .await?;
        }

        Ok(())
    }

    /// Bulk update records in a table and return the updated records.
    pub async fn update_querieable_records<In: Serialize, Out: macros::QueriedRecord>(
        &self,
        records: Vec<Record<In>>,
    ) -> Result<Vec<Out>> {
        let table = Out::Table::TABLE_NAME;
        let (return_fields_by_id, _fields) = Out::queried_fields();

        let mut updated_records = Vec::new();
        let mut records = records.into_iter();
        while records.len() > 0 {
            let chunk = (&mut records).take(10).collect();
            updated_records.extend(
                self.update_records_inner(table, chunk, Some(return_fields_by_id))
                    .await?,
            );
        }

        updated_records
            .into_iter()
            .map(|record| {
                Out::from_record(&record).map_err(|e| Error::ParseResponseFailed {
                    source: e,
                    record: record.clone(),
                })
            })
            .collect::<std::result::Result<Vec<Out>, Error>>()
    }

    /// Bulk update records in a table.
    ///
    /// The provided `records` vector MUST contain at most 10 items, or this function will panic.
    async fn update_records_inner<In: Serialize, Out: DeserializeOwned>(
        &self,
        table: &str,
        records: Vec<Record<In>>,
        return_fields_by_field_id: Option<bool>,
    ) -> Result<Vec<Record<Out>>> {
        assert!(records.len() <= 10);

        // Build the request.
        let request = self.request(
            Method::PATCH,
            table.to_string(),
            APICall {
                records,
                offset: String::new(),
                typecast: Some(true),
                perform_upsert: None,
                return_fields_by_field_id,
            },
            None,
        )?;

        let resp = self.client.execute(request).await?;
        match resp.status() {
            StatusCode::OK => (),
            s => {
                return Err(Error::ApiError {
                    status: s,
                    body: resp.text().await?,
                });
            }
        }

        Ok(resp.json::<APICall<Out>>().await?.records)
    }

    /// Bulk upsert records in a table.
    pub async fn upsert_records<T: Serialize + DeserializeOwned>(
        &self,
        table: &str,
        fields_to_merge_on: Vec<&str>,
        records: Vec<Record<T>>,
    ) -> Result<Vec<Record<T>>> {
        if records.len() <= 10 {
            self.upsert_records_inner(table, &fields_to_merge_on, records)
                .await
        } else {
            let mut res = Vec::new();

            let mut records = records.into_iter();
            while records.len() > 0 {
                let chunk = (&mut records).take(10).collect();
                res.extend(
                    self.upsert_records_inner(table, &fields_to_merge_on, chunk)
                        .await?,
                );
            }

            Ok(res)
        }
    }

    /// Bulk update records in a table.
    ///
    /// The provided `records` vector MUST contain at most 10 items, or this function will panic.
    async fn upsert_records_inner<T: Serialize + DeserializeOwned>(
        &self,
        table: &str,
        fields_to_merge_on: &[&str],
        records: Vec<Record<T>>,
    ) -> Result<Vec<Record<T>>> {
        assert!(records.len() <= 10);

        // Build the request.
        let request = self.request(
            Method::PATCH,
            table.to_string(),
            APICall {
                records,
                offset: String::new(),
                typecast: Some(true),
                perform_upsert: Some(APICallPerformUpsert {
                    fields_to_merge_on: fields_to_merge_on.iter().map(|s| s.to_string()).collect(),
                }),
                return_fields_by_field_id: Some(true),
            },
            None,
        )?;

        let resp = self.client.execute(request).await?;
        match resp.status() {
            StatusCode::OK => (),
            s => {
                return Err(Error::ApiError {
                    status: s,
                    body: resp.text().await?,
                });
            }
        }

        Ok(resp.json::<APICall<T>>().await?.records)
    }
}

impl Airtable<HasEnterpriseId> {
    /// List users.
    /// This is for an enterprise admin to do only.
    /// FROM: <https://airtable.com/api/enterprise>
    pub async fn list_users(&self) -> Result<Vec<User>> {
        // Build the request.
        let request = self.request(
            Method::GET,
            format!(
                "v0/meta/enterpriseAccounts/{}/users",
                self.enterprise_account_id.enterprise_account_id,
            ),
            (),
            Some(vec![("state", "provisioned".to_string())]),
        )?;

        let resp = self.client.execute(request).await?;
        match resp.status() {
            StatusCode::OK => (),
            s => {
                return Err(Error::ApiError {
                    status: s,
                    body: resp.text().await?,
                });
            }
        }

        // Try to deserialize the response.
        let result: UsersResponse = resp.json().await?;

        Ok(result.users)
    }

    /// Get an enterprise user.
    /// This is for an enterprise admin to do only.
    /// FROM: <https://airtable.com/api/enterprise#enterpriseAccountUserGetInformationByEmail>
    /// Permission level can be: owner | create | edit | comment | read
    pub async fn get_enterprise_user(&self, email: &str) -> Result<EnterpriseUser> {
        // Build the request.
        let request = self.request(
            Method::GET,
            format!(
                "v0/meta/enterpriseAccounts/{}/users",
                self.enterprise_account_id.enterprise_account_id,
            ),
            (),
            Some(vec![
                ("email", email.to_string()),
                ("include", "collaborations".to_string()),
            ]),
        )?;

        let resp = self.client.execute(request).await?;

        match resp.status() {
            StatusCode::OK => (),
            s => {
                return Err(Error::ApiError {
                    status: s,
                    body: resp.text().await?,
                });
            }
        }

        let r: EnterpriseUsersResponse = resp.json().await?;

        if r.users.is_empty() {
            return Err(Error::NoUserReturned);
        }

        Ok(r.users.first().unwrap().clone())
    }

    /// Add a collaborator to a workspace.
    /// This is for an enterprise admin to do only.
    /// FROM: <https://airtable.com/api/enterprise#enterpriseWorkspaceAddCollaborator>
    /// Permission level can be: owner | create | edit | comment | read
    pub async fn add_collaborator_to_workspace(
        &self,
        workspace_id: &str,
        user_id: &str,
        permission_level: &str,
    ) -> Result<()> {
        // Build the request.
        let request = self.request(
            Method::POST,
            format!("v0/meta/workspaces/{workspace_id}/collaborators"),
            NewCollaborator {
                collaborators: vec![Collaborator {
                    user: User {
                        id: user_id.to_string(),
                        email: Default::default(),
                        name: Default::default(),
                    },
                    permission_level: permission_level.to_string(),
                }],
            },
            None,
        )?;

        let resp = self.client.execute(request).await?;
        match resp.status() {
            StatusCode::OK => (),
            s => {
                return Err(Error::ApiError {
                    status: s,
                    body: resp.text().await?,
                });
            }
        }

        Ok(())
    }

    /// Returns basic information on the workspace. Does not include deleted collaborators
    /// and only include outstanding invites.
    /// FROM: <https://airtable.com/api/enterprise#enterpriseWorkspaceGetInformation>
    pub async fn get_enterprise_workspace<const N: usize>(
        &self,
        workspace_id: &str,
        includes: Option<[WorkspaceIncludes; N]>,
    ) -> Result<Workspace> {
        // Build the request.
        let request = self.request(
            Method::GET,
            format!("v0/meta/workspaces/{workspace_id}?"),
            (),
            includes.map(|includes| {
                includes
                    .map(|include| {
                        (
                            "include",
                            match include {
                                WorkspaceIncludes::Collaborators => "collaborators".to_string(),
                                WorkspaceIncludes::InviteLinks => "inviteLinks".to_string(),
                            },
                        )
                    })
                    .to_vec()
            }),
        )?;

        let resp = self.client.execute(request).await?;
        match resp.status() {
            StatusCode::OK => (),
            s => {
                return Err(Error::ApiError {
                    status: s,
                    body: resp.text().await?,
                });
            }
        }

        let r: Workspace = resp.json().await?;

        Ok(r)
    }

    /// Delete internal user by email.
    /// This is for an enterprise admin to do only.
    /// The user must be an internal user, meaning they have an email with the company domain.
    /// FROM: <https://airtable.com/api/enterprise#enterpriseAccountUserDeleteUserByEmail>
    pub async fn delete_internal_user_by_email(&self, email: &str) -> Result<()> {
        // Build the request.
        let request = self.request(
            Method::DELETE,
            format!(
                "v0/meta/enterpriseAccounts/{}/users",
                self.enterprise_account_id.enterprise_account_id,
            ),
            (),
            Some(vec![("email", email.to_string())]),
        )?;

        let resp = self.client.execute(request).await?;
        match resp.status() {
            StatusCode::OK => (),
            s => {
                return Err(Error::ApiError {
                    status: s,
                    body: resp.text().await?,
                });
            }
        }

        // Try to deserialize the response.
        let result: DeleteUserResponse = resp.json().await?;
        if !result.errors.is_empty() {
            return Err(Error::DeleteUserFailed(result));
        }

        Ok(())
    }
}

pub struct Pages<'a, T, HEIM: HasEnterpriseIdMarker = DoesNotHaveEnterpriseId> {
    client: &'a Airtable<HEIM>,
    builder: &'a list_builder::ListRecords,
    offset: Option<String>,
    record_type: PhantomData<T>,
}

impl<'a, T, HEIM> Pages<'a, T, HEIM>
where
    T: DeserializeOwned,
    HEIM: HasEnterpriseIdMarker,
{
    #[must_use]
    pub fn new(client: &'a Airtable<HEIM>, builder: &'a list_builder::ListRecords) -> Self {
        Self {
            client,
            builder,
            offset: Some(String::new()),
            record_type: PhantomData,
        }
    }

    pub async fn next(&mut self) -> Result<Option<Vec<Record<T>>>> {
        if self.offset.is_none() {
            log::debug!("[airtable-api] Page does not have an offset. Returning.");
            return Ok(None);
        }

        let mut params = vec![("pageSize", "100".to_string())];
        if let Some(view) = &self.builder.view_name {
            params.push(("view", view.clone()));
        }
        for field in &self.builder.fields {
            params.push(("fields[]", field.to_string()));
        }
        if let Some(max_records) = self.builder.max_records {
            params.push(("maxRecords", max_records.to_string()));
        }
        if let Some(filter_formula) = &self.builder.filter_formula {
            let filter_formula = filter_formula.to_formula()?;
            params.push(("filterByFormula", filter_formula));
        }
        params.push((
            "returnFieldsByFieldId",
            self.builder.return_fields_by_id.to_string(),
        ));

        if let Some(offset) = &self.offset {
            if !offset.is_empty() {
                log::debug!("[airtable-api] Fetching page of results with offset {offset}");
                params.push(("offset", offset.clone()));
            }
        } else {
            log::debug!("[airtable-api] Requesting first page of records");
        }

        // Build the request.
        let request = self.client.request(
            Method::GET,
            self.builder.table_name.clone(),
            (),
            Some(params),
        )?;

        let response = self.client.client.execute(request).await?;

        match response.status() {
            StatusCode::OK => {
                let api_response: APICall<T> = response.json().await?;
                log::debug!("[airtable-api] Retrieved page response");

                self.offset = if api_response.offset.is_empty() {
                    None
                } else {
                    Some(api_response.offset)
                };

                Ok(Some(api_response.records))
            }
            s => {
                log::debug!(
                    "[airtable-api] Pagination request returned an error. Stopping requests."
                );

                // Once we hit an error we stop pagination
                self.offset = None;

                Err(Error::ApiError {
                    status: s,
                    body: response.text().await?,
                })
            }
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct APICall<T> {
    /// If there are more records, the response will contain an
    /// offset. To fetch the next page of records, include offset
    /// in the next request's parameters.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub offset: String,
    /// The current page number of returned records.
    pub records: Vec<Record<T>>,
    /// The Airtable API will perform best-effort automatic data conversion
    /// from string values if the typecast parameter is passed in. Automatic
    /// conversion is disabled by default to ensure data integrity, but it may
    /// be helpful for integrating with 3rd party data sources.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub typecast: Option<bool>,
    /// Enables upsert behavior when set. Available with PATCH and PUT method.
    ///
    /// See https://airtable.com/developers/web/api/update-multiple-records for more details.
    #[serde(skip_serializing_if = "Option::is_none", rename = "performUpsert")]
    pub perform_upsert: Option<APICallPerformUpsert>,
    /// When set on a write request, the response's `fields` objects (and the request's
    /// own `fields` keys) are keyed by field id instead of field name.
    #[serde(
        skip_serializing_if = "Option::is_none",
        rename = "returnFieldsByFieldId"
    )]
    pub return_fields_by_field_id: Option<bool>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct APICallPerformUpsert {
    /// Records that do not include id will use the fields chosen by fieldsToMergeOn as an external ID to match with existing records.
    /// - If zero matches are found, a new record will be created.
    /// - If one match is found, that record will be updated.
    /// - If multiple matches are found, the request will fail.
    ///
    /// Records that include id will ignore fieldsToMergeOn and behave as normal updates. If no record with the given id exists, the request will fail and will not create a new record. The API response for upsert requests will additionally include updatedRecords and createdRecords arrays, indicating which records in the records array already existed and were updated, or did not exist and were created, respectively.
    #[serde(rename = "fieldsToMergeOn")]
    fields_to_merge_on: Vec<String>,
}

/// The outcome of leniently parsing one record: the parsed value, or the failure.
pub type RecordParse<T> = std::result::Result<T, RecordParseFailure>;

/// A record that failed to parse, together with the raw record.
#[derive(Debug)]
pub struct RecordParseFailure {
    pub error: RecordParsingError,
    pub record: Record<serde_json::Value>,
}

/// The schema of a base, as returned by the Meta API.
#[derive(Debug, Clone, Deserialize)]
pub struct BaseSchema {
    pub tables: Vec<TableSchema>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct TableSchema {
    pub id: String,
    pub name: String,
    #[serde(rename = "primaryFieldId")]
    pub primary_field_id: String,
    pub fields: Vec<FieldSchema>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct FieldSchema {
    pub id: String,
    pub name: String,
}

/// An Airtable record.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Record<T> {
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub id: String,
    pub fields: T,
    #[serde(rename = "createdTime", skip_serializing)]
    pub created_time: Option<DateTime<Utc>>,
}

/// An airtable user.
#[derive(Debug, Default, Clone, Serialize, JsonSchema, Deserialize)]
pub struct User {
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub id: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub email: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub name: String,
}

enum UserField {
    Id,
    Email,
    Name,
}

const USERFIELDS: &[&str] = &["id", "email", "name"];

impl<'de> Deserialize<'de> for UserField {
    fn deserialize<D>(deserializer: D) -> std::result::Result<UserField, D::Error>
    where
        D: Deserializer<'de>,
    {
        struct UserFieldVisitor;

        impl Visitor<'_> for UserFieldVisitor {
            type Value = UserField;

            fn expecting(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
                formatter.write_str("`id` `email` or `name`")
            }

            fn visit_str<E>(self, value: &str) -> std::result::Result<UserField, E>
            where
                E: serde::de::Error,
            {
                match value {
                    "id" => Ok(UserField::Id),
                    "email" => Ok(UserField::Email),
                    "name" => Ok(UserField::Name),
                    _ => Err(serde::de::Error::unknown_field(value, USERFIELDS)),
                }
            }
        }

        deserializer.deserialize_identifier(UserFieldVisitor)
    }
}

struct UserVisitor;

impl<'de> Visitor<'de> for UserVisitor {
    type Value = User;

    fn expecting(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
        formatter.write_str("struct User")
    }

    fn visit_seq<V>(self, mut seq: V) -> std::result::Result<User, V::Error>
    where
        V: SeqAccess<'de>,
    {
        let id = seq
            .next_element()?
            .ok_or_else(|| serde::de::Error::invalid_length(0, &self))?;
        let email = seq
            .next_element()?
            .ok_or_else(|| serde::de::Error::invalid_length(1, &self))?;
        let name = seq
            .next_element()?
            .ok_or_else(|| serde::de::Error::invalid_length(2, &self))?;
        Ok(User { id, email, name })
    }

    fn visit_map<V>(self, mut map: V) -> std::result::Result<User, V::Error>
    where
        V: MapAccess<'de>,
    {
        let mut id = None;
        let mut email = None;
        let mut name = None;
        while let Some(key) = map.next_key()? {
            match key {
                UserField::Id => {
                    if id.is_some() {
                        return Err(serde::de::Error::duplicate_field("id"));
                    }
                    id = Some(map.next_value()?);
                }
                UserField::Email => {
                    if email.is_some() {
                        return Err(serde::de::Error::duplicate_field("email"));
                    }
                    email = Some(map.next_value()?);
                }
                UserField::Name => {
                    if name.is_some() {
                        return Err(serde::de::Error::duplicate_field("name"));
                    }
                    name = Some(map.next_value()?);
                }
            }
        }
        let id = id.unwrap_or_default();
        let email = email.ok_or_else(|| serde::de::Error::missing_field("email"))?;
        let name = name.unwrap_or_default();
        Ok(User { id, email, name })
    }
}

struct UsersVisitor;

impl<'de> Visitor<'de> for UsersVisitor {
    type Value = Vec<User>;

    fn expecting(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
        formatter.write_str("a very special vector")
    }

    fn visit_seq<A: SeqAccess<'de>>(
        self,
        mut access: A,
    ) -> std::result::Result<Self::Value, A::Error> {
        let mut users: Vec<User> = Default::default();

        // While there are entries remaining in the input, add them
        // into our vector.
        while let Some(user) = access.next_element::<User>()? {
            users.push(user);
        }

        Ok(users)
    }
}

/// The response returned from listing users.
#[derive(Debug, Default, Clone, Serialize, Deserialize)]
pub struct UsersResponse {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub users: Vec<User>,
}

/// The response returned from deleting a user.
/// FROM: <https://airtable.com/api/enterprise#enterpriseAccountUserDeleteUserByEmail>
#[derive(Debug, Default, Clone, Serialize, Deserialize)]
pub struct DeleteUserResponse {
    #[serde(
        default,
        skip_serializing_if = "Vec::is_empty",
        rename = "deletedUsers"
    )]
    pub deleted_users: Vec<User>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub errors: Vec<ErrorResponse>,
}

#[derive(Debug, Default, Clone, Serialize, Deserialize)]
pub struct ErrorResponse {
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub email: String,
    #[serde(default, skip_serializing_if = "String::is_empty", rename = "type")]
    pub type_: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub message: String,
}

#[derive(Debug, Default, Clone, Serialize, Deserialize)]
pub struct AttachmentShort {
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub url: String,
}

#[derive(Debug, Default, Clone, Serialize, Deserialize)]
pub struct Attachment {
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub id: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub url: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub filename: String,
    #[serde(default)]
    pub size: i64,
    #[serde(default, skip_serializing_if = "String::is_empty", rename = "type")]
    pub type_: String,
    #[serde(default)]
    pub thumbnails: Thumbnails,
}

#[derive(Debug, Default, Clone, Serialize, Deserialize)]
pub struct Thumbnails {
    #[serde(default)]
    pub small: Full,
    #[serde(default)]
    pub large: Full,
    #[serde(default)]
    pub full: Full,
}

#[derive(Debug, Default, Clone, Serialize, Deserialize)]
pub struct Full {
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub url: String,
    #[serde(default)]
    pub width: i64,
    #[serde(default)]
    pub height: i64,
}

#[derive(Debug, Default, Clone, Serialize, Deserialize)]
pub struct NewCollaborator {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub collaborators: Vec<Collaborator>,
}

#[derive(Debug, Default, Clone, Serialize, Deserialize)]
pub struct Collaborator {
    #[serde(default)]
    pub user: User,
    #[serde(
        default,
        skip_serializing_if = "String::is_empty",
        deserialize_with = "deserialize_null_string::deserialize",
        rename = "permissionLevel"
    )]
    pub permission_level: String,
}

#[derive(Debug, Default, Clone, Serialize, Deserialize)]
pub struct EnterpriseUsersResponse {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub users: Vec<EnterpriseUser>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EnterpriseUser {
    #[serde(
        default,
        skip_serializing_if = "String::is_empty",
        deserialize_with = "deserialize_null_string::deserialize"
    )]
    pub id: String,
    #[serde(
        default,
        skip_serializing_if = "String::is_empty",
        deserialize_with = "deserialize_null_string::deserialize"
    )]
    pub state: String,
    #[serde(
        default,
        skip_serializing_if = "String::is_empty",
        deserialize_with = "deserialize_null_string::deserialize"
    )]
    pub email: String,
    #[serde(
        default,
        skip_serializing_if = "String::is_empty",
        deserialize_with = "deserialize_null_string::deserialize"
    )]
    pub name: String,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        rename = "lastActivityTime",
        deserialize_with = "deserialize_missing_timezone::deserialize"
    )]
    pub last_activity_time: Option<DateTime<Utc>>,
    #[serde(
        default,
        skip_serializing_if = "String::is_empty",
        deserialize_with = "deserialize_null_string::deserialize",
        rename = "invitedToAirtableByUserId"
    )]
    pub invited_to_airtable_by_user_id: String,
    #[serde(rename = "createdTime")]
    pub created_time: DateTime<Utc>,
    #[serde(default)]
    pub collaborations: Collaborations,
}

#[derive(Debug, Default, Clone, Serialize, Deserialize)]
pub struct Collaborations {
    #[serde(
        default,
        skip_serializing_if = "Vec::is_empty",
        rename = "workspaceCollaborations"
    )]
    pub workspace_collaborations: Vec<Collaboration>,
    #[serde(
        default,
        skip_serializing_if = "Vec::is_empty",
        rename = "baseCollaborations"
    )]
    pub base_collaborations: Vec<Collaboration>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Collaboration {
    #[serde(
        default,
        skip_serializing_if = "String::is_empty",
        deserialize_with = "deserialize_null_string::deserialize",
        rename = "baseId"
    )]
    pub base_id: String,
    #[serde(
        default,
        skip_serializing_if = "String::is_empty",
        deserialize_with = "deserialize_null_string::deserialize",
        rename = "permissionLevel"
    )]
    pub permission_level: String,
    #[serde(rename = "createdTime")]
    pub created_time: DateTime<Utc>,
    #[serde(
        default,
        skip_serializing_if = "String::is_empty",
        deserialize_with = "deserialize_null_string::deserialize",
        rename = "grantedByUserId"
    )]
    pub granted_by_user_id: String,
    #[serde(
        default,
        skip_serializing_if = "String::is_empty",
        deserialize_with = "deserialize_null_string::deserialize",
        rename = "workspaceId"
    )]
    pub workspace_id: String,
}

/// Optional include flags that can be passed to [`get_enterprise_workspace`] to control
/// fields are returned
pub enum WorkspaceIncludes {
    Collaborators,
    InviteLinks,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Workspace {
    pub id: String,
    pub name: String,
    pub created_time: Option<DateTime<Utc>>,
    #[serde(rename = "baseIds")]
    pub base_ids: Vec<String>,
    #[serde(rename = "individualCollaborators")]
    pub individual_collaborators: Option<WorkspaceCollaborators>,
    #[serde(rename = "baseCollaborators")]
    pub group_collaborators: Option<WorkspaceCollaborators>,
    pub invite_links: Option<InviteLinks>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkspaceCollaborators {
    #[serde(rename = "workspaceCollaborators")]
    pub workspace_collaborators: Vec<WorkspaceCollaborator>,
    #[serde(rename = "baseCollaborators")]
    pub base_collaborators: Vec<BaseCollaborator>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkspaceCollaborator {
    #[serde(rename = "userId")]
    pub user_id: String,
    pub email: String,
    #[serde(rename = "permissionLevel")]
    pub permission_level: String,
    #[serde(rename = "createdTime")]
    pub created_time: Option<DateTime<Utc>>,
    #[serde(rename = "grantedByUserId")]
    pub granted_by_user_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BaseCollaborator {
    #[serde(rename = "baseId")]
    pub base_id: String,
    #[serde(rename = "userId")]
    pub user_id: String,
    pub email: String,
    #[serde(rename = "permissionLevel")]
    pub permission_level: String,
    #[serde(rename = "createdTime")]
    pub created_time: Option<DateTime<Utc>>,
    #[serde(rename = "grantedByUserId")]
    pub granted_by_user_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InviteLinks {
    pub workspace_invite_links: Vec<WorkspaceInviteLink>,
    pub base_invite_links: Vec<BaseInviteLink>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkspaceInviteLink {
    pub id: String,
    #[serde(rename = "type")]
    pub _type: String,
    #[serde(rename = "invitedEmail")]
    pub invited_email: String,
    #[serde(rename = "restrictedToEmailDomains")]
    pub restricted_to_email_domains: Vec<String>,
    #[serde(rename = "createdTime")]
    pub created_time: Option<DateTime<Utc>>,
    #[serde(rename = "permissionLevel")]
    pub permission_level: String,
    #[serde(rename = "referredByUserId")]
    pub referred_by_user_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BaseInviteLink {
    pub id: String,
    #[serde(rename = "baseId")]
    pub base_id: String,
    #[serde(rename = "type")]
    pub _type: String,
    #[serde(rename = "invitedEmail")]
    pub invited_email: String,
    #[serde(rename = "restrictedToEmailDomains")]
    pub restricted_to_email_domains: Vec<String>,
    #[serde(rename = "createdTime")]
    pub created_time: Option<DateTime<Utc>>,
    #[serde(rename = "permissionLevel")]
    pub permission_level: String,
    #[serde(rename = "referredByUserId")]
    pub referred_by_user_id: String,
}

struct AttachmentsVisitor;

impl<'de> Visitor<'de> for AttachmentsVisitor {
    type Value = Vec<Attachment>;

    fn expecting(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
        formatter.write_str("a very special vector")
    }

    fn visit_seq<A: SeqAccess<'de>>(
        self,
        mut access: A,
    ) -> std::result::Result<Self::Value, A::Error> {
        let mut attachments: Vec<Attachment> = Default::default();

        // While there are entries remaining in the input, add them
        // into our vector.
        while let Some(attachment) = access.next_element::<Attachment>()? {
            attachments.push(attachment);
        }

        Ok(attachments)
    }
}

pub mod user_format_as_array_of_strings {
    use serde::{self, ser::SerializeSeq, Deserializer, Serializer};

    use super::{User, UsersVisitor};

    // The signature of a serialize_with function must follow the pattern:
    //
    //    fn serialize<S>(&T, S) -> std::result::Result<S::Ok, S::Error>
    //    where
    //        S: Serializer
    //
    // although it may also be generic over the input types T.
    pub fn serialize<S>(array: &[String], serializer: S) -> std::result::Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        // Make our array of Airtable user objects.
        let mut seq = serializer.serialize_seq(Some(array.len())).unwrap();
        for e in array {
            seq.serialize_element(&User {
                id: Default::default(),
                email: e.clone(),
                name: Default::default(),
            })
            .unwrap();
        }
        seq.end()
    }

    // The signature of a deserialize_with function must follow the pattern:
    //
    //    fn deserialize<'de, D>(D) -> std::result::Result<T, D::Error>
    //    where
    //        D: Deserializer<'de>
    //
    // although it may also be generic over the output types T.
    pub fn deserialize<'de, D>(deserializer: D) -> Result<Vec<String>, D::Error>
    where
        D: Deserializer<'de>,
    {
        let airtable_users = deserializer.deserialize_seq(UsersVisitor {}).unwrap();

        let mut users: Vec<String> = Default::default();
        for a in airtable_users {
            users.push(a.email.clone());
        }

        Ok(users)
    }
}

pub mod user_format_as_string {
    use serde::{self, ser::SerializeStruct, Deserializer, Serializer};

    use super::{UserVisitor, USERFIELDS};

    // The signature of a serialize_with function must follow the pattern:
    //
    //    fn serialize<S>(&T, S) -> std::result::Result<S::Ok, S::Error>
    //    where
    //        S: Serializer
    //
    // although it may also be generic over the input types T.
    pub fn serialize<S>(email: &str, serializer: S) -> std::result::Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        let mut state = serializer.serialize_struct("User", 1)?;
        state.serialize_field("email", &email)?;
        state.end()
    }

    // The signature of a deserialize_with function must follow the pattern:
    //
    //    fn deserialize<'de, D>(D) -> std::result::Result<T, D::Error>
    //    where
    //        D: Deserializer<'de>
    //
    // although it may also be generic over the output types T.
    pub fn deserialize<'de, D>(deserializer: D) -> std::result::Result<String, D::Error>
    where
        D: Deserializer<'de>,
    {
        let user = deserializer
            .deserialize_struct("User", USERFIELDS, UserVisitor)
            .unwrap();
        Ok(user.email)
    }
}

pub mod attachment_format_as_array_of_strings {
    use serde::{self, ser::SerializeSeq, Deserializer, Serializer};

    use super::{AttachmentShort, AttachmentsVisitor};

    // The signature of a serialize_with function must follow the pattern:
    //
    //    fn serialize<S>(&T, S) -> std::result::Result<S::Ok, S::Error>
    //    where
    //        S: Serializer
    //
    // although it may also be generic over the input types T.
    pub fn serialize<S>(array: &[String], serializer: S) -> std::result::Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        // Make our array of Airtable attachment objects.
        let mut seq = serializer.serialize_seq(Some(array.len())).unwrap();
        for e in array {
            let mut attachment: AttachmentShort = Default::default();
            attachment.url = e.clone();
            seq.serialize_element(&attachment).unwrap();
        }
        seq.end()
    }

    // The signature of a deserialize_with function must follow the pattern:
    //
    //    fn deserialize<'de, D>(D) -> std::result::Result<T, D::Error>
    //    where
    //        D: Deserializer<'de>
    //
    // although it may also be generic over the output types T.
    pub fn deserialize<'de, D>(deserializer: D) -> Result<Vec<String>, D::Error>
    where
        D: Deserializer<'de>,
    {
        let airtable_attachments = deserializer.deserialize_seq(AttachmentsVisitor {}).unwrap();

        let mut attachments: Vec<String> = Default::default();
        for a in airtable_attachments {
            attachments.push(a.url.clone());
        }

        Ok(attachments)
    }
}

pub mod attachment_format_as_string {
    use serde::{self, ser::SerializeSeq, Deserializer, Serializer};

    use super::{Attachment, AttachmentsVisitor};

    // The signature of a serialize_with function must follow the pattern:
    //
    //    fn serialize<S>(&T, S) -> std::result::Result<S::Ok, S::Error>
    //    where
    //        S: Serializer
    //
    // although it may also be generic over the input types T.
    pub fn serialize<S>(url: &str, serializer: S) -> std::result::Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        // Make our array of Airtable attachment objects.
        let mut seq = serializer.serialize_seq(Some(1)).unwrap();
        let mut attachment: Attachment = Default::default();
        attachment.url = url.to_string();
        seq.serialize_element(&attachment).unwrap();
        seq.end()
    }

    // The signature of a deserialize_with function must follow the pattern:
    //
    //    fn deserialize<'de, D>(D) -> std::result::Result<T, D::Error>
    //    where
    //        D: Deserializer<'de>
    //
    // although it may also be generic over the output types T.
    pub fn deserialize<'de, D>(deserializer: D) -> std::result::Result<String, D::Error>
    where
        D: Deserializer<'de>,
    {
        let airtable_attachments = deserializer.deserialize_seq(AttachmentsVisitor {}).unwrap();
        let mut url = String::new();
        if !airtable_attachments.is_empty() {
            url = airtable_attachments[0].url.clone();
        }
        Ok(url)
    }
}

/// An airtable barcode.
#[derive(Debug, Default, Clone, Serialize, JsonSchema, Deserialize)]
pub struct Barcode {
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub text: String,
    #[serde(default, skip_serializing_if = "String::is_empty", rename = "type")]
    pub type_: String,
}

struct BarcodeVisitor;

impl<'de> Visitor<'de> for BarcodeVisitor {
    type Value = Barcode;

    fn expecting(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
        formatter.write_str("struct Barcode")
    }

    fn visit_seq<V>(self, mut seq: V) -> std::result::Result<Barcode, V::Error>
    where
        V: SeqAccess<'de>,
    {
        let text = seq
            .next_element()?
            .ok_or_else(|| serde::de::Error::invalid_length(0, &self))?;
        let type_ = seq
            .next_element()?
            .ok_or_else(|| serde::de::Error::invalid_length(1, &self))?;
        Ok(Barcode { text, type_ })
    }

    fn visit_map<V>(self, mut map: V) -> std::result::Result<Barcode, V::Error>
    where
        V: MapAccess<'de>,
    {
        let mut text = None;
        let mut type_ = None;
        while let Some(key) = map.next_key()? {
            match key {
                BarcodeField::Text => {
                    if text.is_some() {
                        return Err(serde::de::Error::duplicate_field("text"));
                    }
                    text = Some(map.next_value()?);
                }
                BarcodeField::Type => {
                    if type_.is_some() {
                        return Err(serde::de::Error::duplicate_field("type"));
                    }
                    type_ = Some(map.next_value()?);
                }
            }
        }
        let text = text.ok_or_else(|| serde::de::Error::missing_field("text"))?;
        let type_ = type_.ok_or_else(|| serde::de::Error::missing_field("type"))?;
        Ok(Barcode { text, type_ })
    }
}

enum BarcodeField {
    Text,
    Type,
}

const BARCODEFIELDS: &[&str] = &["text", "type"];

impl<'de> Deserialize<'de> for BarcodeField {
    fn deserialize<D>(deserializer: D) -> std::result::Result<BarcodeField, D::Error>
    where
        D: Deserializer<'de>,
    {
        struct BarcodeFieldVisitor;

        impl Visitor<'_> for BarcodeFieldVisitor {
            type Value = BarcodeField;

            fn expecting(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
                formatter.write_str("`text` or `type`")
            }

            fn visit_str<E>(self, value: &str) -> std::result::Result<BarcodeField, E>
            where
                E: serde::de::Error,
            {
                match value {
                    "text" => Ok(BarcodeField::Text),
                    "type" => Ok(BarcodeField::Type),
                    _ => Err(serde::de::Error::unknown_field(value, BARCODEFIELDS)),
                }
            }
        }

        deserializer.deserialize_identifier(BarcodeFieldVisitor)
    }
}

pub mod barcode_format_as_string {
    use serde::{self, ser::SerializeStruct, Deserializer, Serializer};

    use super::{BarcodeVisitor, BARCODEFIELDS};

    // The signature of a serialize_with function must follow the pattern:
    //
    //    fn serialize<S>(&T, S) -> std::result::Result<S::Ok, S::Error>
    //    where
    //        S: Serializer
    //
    // although it may also be generic over the input types T.
    pub fn serialize<S>(text: &str, serializer: S) -> std::result::Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        let mut state = serializer.serialize_struct("Barcode", 1)?;
        state.serialize_field("text", &text)?;
        // This needs to be code39 or upce.
        state.serialize_field("type", "code39")?;
        state.end()
    }

    // The signature of a deserialize_with function must follow the pattern:
    //
    //    fn deserialize<'de, D>(D) -> std::result::Result<T, D::Error>
    //    where
    //        D: Deserializer<'de>
    //
    // although it may also be generic over the output types T.
    pub fn deserialize<'de, D>(deserializer: D) -> std::result::Result<String, D::Error>
    where
        D: Deserializer<'de>,
    {
        let barcode = deserializer
            .deserialize_struct("Barcode", BARCODEFIELDS, BarcodeVisitor)
            .unwrap();
        Ok(barcode.text)
    }
}

pub mod deserialize_null_string {
    use serde::{self, Deserialize, Deserializer};

    // The signature of a deserialize_with function must follow the pattern:
    //
    //    fn deserialize<'de, D>(D) -> std::result::Result<T, D::Error>
    //    where
    //        D: Deserializer<'de>
    //
    // although it may also be generic over the output types T.
    pub fn deserialize<'de, D>(deserializer: D) -> std::result::Result<String, D::Error>
    where
        D: Deserializer<'de>,
    {
        let s = String::deserialize(deserializer).unwrap_or_default();

        Ok(s)
    }
}

// The Airtable API started returning EnterpriseUser results with timestamps that do
// not specify a timezone. We assume these to be Utc
pub mod deserialize_missing_timezone {
    use chrono::{DateTime, Utc};
    use serde::{self, de::Error, Deserialize, Deserializer};

    // The signature of a deserialize_with function must follow the pattern:
    //
    //    fn deserialize<'de, D>(D) -> std::result::Result<T, D::Error>
    //    where
    //        D: Deserializer<'de>
    //
    // although it may also be generic over the output types T.
    pub fn deserialize<'de, D>(deserializer: D) -> Result<Option<DateTime<Utc>>, D::Error>
    where
        D: Deserializer<'de>,
    {
        // Airtable can have issues with timezones, so try to deserialize to a
        // string instead for parsing
        let mut s = String::deserialize(deserializer).unwrap_or_default();

        DateTime::parse_from_rfc3339(s.as_str())
            .or_else(|_| {
                // The initial parse failed, so instead we can try to fix the incoming
                // string representation and parse again. If this constructs yet another
                // invalid timestamp, then deserialization will fail
                s += "Z";
                DateTime::parse_from_rfc3339(s.as_str())
            })
            .map(|fixed_offset| Some(fixed_offset.with_timezone(&Utc)))
            .map_err(D::Error::custom)
    }
}
