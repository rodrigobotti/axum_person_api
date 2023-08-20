use std::error::Error;
use std::sync::Arc;

use axum::extract::{Path, Query, State};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{async_trait, Json, Router};
use chrono::NaiveDate;
use hyper::StatusCode;
use serde::{Deserialize, Serialize};
use sqlx::postgres::{PgDatabaseError, PgPoolOptions};
use sqlx::{Pool, Postgres};

#[derive(Debug, Deserialize, Serialize, Default, sqlx::FromRow)]
struct Person {
    pub id: i64,
    #[serde(rename(serialize = "apelido"))]
    pub nickname: String,
    #[serde(rename(serialize = "nome"))]
    pub name: String,
    #[serde(rename(serialize = "nascimento"))]
    pub dob: NaiveDate,
    #[serde(rename(serialize = "stack"))]
    pub stacks: Option<Vec<String>>,
}

#[derive(Debug, Deserialize, Serialize, Default)]
struct CreatePersonPayload {
    #[serde(rename(deserialize = "apelido"))]
    pub nickname: String,
    #[serde(rename(deserialize = "nome"))]
    pub name: String,
    #[serde(rename(deserialize = "nascimento"))]
    pub dob: NaiveDate,
    #[serde(rename(deserialize = "stack"))]
    pub stacks: Option<Vec<String>>,
}

#[derive(Debug, Default, Deserialize)]
struct SearchPersonQuery {
    t: String,
}

// TODO: serialize json parse errors as ErrorResponse

#[derive(Debug)]
enum RepositoryError {
    #[allow(dead_code)]
    NotFound {
        resoure_name: &'static str,
        resource_id: i64,
    },
    #[allow(dead_code)]
    Conflict { reason: String },
    #[allow(dead_code)]
    Unexpected,
}

impl IntoResponse for RepositoryError {
    fn into_response(self) -> Response {
        let status = match self {
            RepositoryError::NotFound { .. } => StatusCode::NOT_FOUND,
            RepositoryError::Conflict { .. } => StatusCode::UNPROCESSABLE_ENTITY,
            RepositoryError::Unexpected => StatusCode::INTERNAL_SERVER_ERROR,
        };
        (status, Json(ErrorResponse::from(self))).into_response()
    }
}

impl From<RepositoryError> for AppError {
    fn from(value: RepositoryError) -> Self {
        AppError::Repo(value)
    }
}

impl From<RepositoryError> for ErrorResponse {
    fn from(value: RepositoryError) -> Self {
        match value {
            RepositoryError::NotFound {
                resoure_name,
                resource_id,
            } => ErrorResponse {
                detail: format!(
                    "Resource {} with id {} not found",
                    resoure_name, resource_id
                ),
                o_type: "NotFound",
                title: "Resource not found",
                status: StatusCode::NOT_FOUND.as_u16(),
            },
            RepositoryError::Conflict { reason } => ErrorResponse {
                status: StatusCode::UNPROCESSABLE_ENTITY.as_u16(),
                o_type: "Conflict",
                title: "Unprocessable entity",
                detail: format!("Conflict due to {}", reason),
            },
            RepositoryError::Unexpected => ErrorResponse {
                status: StatusCode::INTERNAL_SERVER_ERROR.as_u16(),
                o_type: "Unexpected",
                title: "Internal Server Error",
                detail: "Unexpected error".to_owned(),
            },
        }
    }
}

enum AppError {
    Repo(RepositoryError),
}

#[derive(Serialize)]
struct ErrorResponse {
    pub status: u16,
    #[serde(rename = "type")]
    pub o_type: &'static str,
    pub title: &'static str,
    pub detail: String,
}

impl IntoResponse for AppError {
    fn into_response(self) -> Response {
        match self {
            AppError::Repo(inner) => inner.into_response(),
        }
    }
}

#[async_trait]
trait PersonRepository {
    async fn create_person(&self, person: CreatePersonPayload) -> Result<Person, AppError>;
    async fn get_person(&self, id: i64) -> Result<Person, AppError>;
    async fn search_person(&self, term: String) -> Result<Vec<Person>, AppError>;
    async fn count(&self) -> Result<i64, AppError>;
}

struct PostgresPersonRepository {
    pool: Pool<Postgres>,
}

impl PostgresPersonRepository {
    fn new(pool: Pool<Postgres>) -> Self {
        PostgresPersonRepository { pool }
    }
}

impl PostgresPersonRepository {
    fn handle_create_error(err: sqlx::Error) -> AppError {
        if let Some(pg_error) = err
            .as_database_error()
            .map(|e| e.downcast_ref::<PgDatabaseError>())
        {
            if pg_error.code() == "23505" {
                return RepositoryError::Conflict {
                    reason: "nickname already taken".to_owned(),
                }
                .into();
            }
        }
        // TODO: log error
        RepositoryError::Unexpected.into()
    }
}

#[async_trait]
impl PersonRepository for PostgresPersonRepository {
    async fn create_person(&self, person: CreatePersonPayload) -> Result<Person, AppError> {
        sqlx::query_as(
            "INSERT INTO person (nickname, name, dob, stacks) 
            VALUES ($1, $2, $3, $4)
            RETURNING *",
        )
        .bind(person.nickname)
        .bind(person.name)
        .bind(person.dob)
        .bind(person.stacks)
        .fetch_one(&self.pool)
        .await
        .map_err(Self::handle_create_error)
    }

    async fn get_person(&self, id: i64) -> Result<Person, AppError> {
        let result = sqlx::query_as("SELECT * FROM person WHERE id = $1")
            .bind(id)
            .fetch_optional(&self.pool)
            .await;

        match result {
            Ok(Some(person)) => Ok(person),
            Ok(None) => Err(RepositoryError::NotFound {
                resoure_name: "person",
                resource_id: id,
            }
            .into()),
            Err(_err) => {
                // TODO: log error
                Err(RepositoryError::Unexpected.into())
            }
        }
    }
    // TODO: performance
    async fn search_person(&self, term: String) -> Result<Vec<Person>, AppError> {
        let search_term = format!("%{term}%");

        sqlx::query_as(
            "SELECT * FROM person
            WHERE 
                nickname LIKE $1
                OR name LIKE $1
                OR EXISTS (
                    SELECT 1 FROM UNNEST(stacks) s
                    WHERE s LIKE $1
                )
            LIMIT 50",
        )
        .bind(search_term)
        .fetch_all(&self.pool)
        .await
        .map_err(|_err| {
            // TODO: log error
            RepositoryError::Unexpected.into()
        })
    }

    async fn count(&self) -> Result<i64, AppError> {
        sqlx::query_scalar("SELECT COUNT(*) FROM person")
            .fetch_one(&self.pool)
            .await
            .map_err(|_err| {
                // TODO: log error
                RepositoryError::Unexpected.into()
            })
    }
}

type DynPersonRepo = Arc<dyn PersonRepository + Send + Sync>;

async fn get_person(
    Path(id): Path<i64>,
    State(repo): State<DynPersonRepo>,
) -> Result<Json<Person>, AppError> {
    let person = repo.get_person(id).await?;
    Ok(person.into())
}

async fn create_person(
    State(repo): State<DynPersonRepo>,
    Json(payload): Json<CreatePersonPayload>,
) -> Result<Json<Person>, AppError> {
    let person = repo.create_person(payload).await?;
    Ok(person.into())
}

async fn search_person(
    Query(query): Query<SearchPersonQuery>,
    State(repo): State<DynPersonRepo>,
) -> Result<Json<Vec<Person>>, AppError> {
    let ps = repo.search_person(query.t).await?;
    Ok(ps.into())
}

async fn count_person(State(repo): State<DynPersonRepo>) -> Result<String, AppError> {
    let count = repo.count().await?;
    Ok(count.to_string())
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn Error>> {
    // TODO: log + tracing using trace

    println!("Connecting to database");
    let conn_string = "postgres://person:person@localhost:5432/person";
    let pool = PgPoolOptions::new().connect(conn_string).await?;
    let repo: DynPersonRepo = Arc::new(PostgresPersonRepository::new(pool));

    println!("Starting server");

    let app = Router::new()
        .route("/pessoas/:id", get(get_person))
        .route("/pessoas", post(create_person))
        .route("/pessoas", get(search_person))
        .route("/contagem-pessoas", get(count_person))
        .with_state(repo);

    axum::Server::bind(&"0.0.0.0:3000".parse().unwrap())
        .serve(app.into_make_service())
        .await
        .expect("Failed to start service");

    println!("Server started");

    Ok(())
}