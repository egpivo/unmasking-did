use anyhow::{Context, Result};
use sqlx::sqlite::{SqliteConnectOptions, SqlitePoolOptions};
use sqlx::{Row, SqlitePool};
use std::str::FromStr;

use crate::alchemy::Transfer;

const EMBEDDED_SCHEMA: &str = include_str!("schema.sql");

pub async fn connect(database_url: &str) -> Result<SqlitePool> {
    let opts = SqliteConnectOptions::from_str(database_url)
        .with_context(|| format!("invalid SQLite URL: {database_url}"))?
        .create_if_missing(true);

    SqlitePoolOptions::new()
        .max_connections(5)
        .connect_with(opts)
        .await
        .context("failed to connect to SQLite")
}

pub async fn run_migrations(pool: &SqlitePool) -> Result<()> {
    for stmt in split_statements(EMBEDDED_SCHEMA) {
        sqlx::query(&stmt)
            .execute(pool)
            .await
            .with_context(|| format!("failed to run schema statement: {stmt}"))?;
    }
    Ok(())
}

fn split_statements(sql: &str) -> Vec<String> {
    sql.split(';')
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(|s| s.to_string())
        .collect()
}

#[derive(Clone)]
pub struct Repo {
    pool: SqlitePool,
}

impl Repo {
    pub fn new(pool: SqlitePool) -> Self {
        Self { pool }
    }

    pub fn pool(&self) -> &SqlitePool {
        &self.pool
    }

    pub async fn upsert_address(&self, address: &str, first_seen_block: Option<i64>) -> Result<()> {
        sqlx::query(
            "INSERT INTO addresses (address, first_seen_block) VALUES (?1, ?2)
             ON CONFLICT(address) DO UPDATE SET
               first_seen_block = CASE
                   WHEN excluded.first_seen_block IS NULL THEN addresses.first_seen_block
                   WHEN addresses.first_seen_block IS NULL THEN excluded.first_seen_block
                   WHEN excluded.first_seen_block < addresses.first_seen_block
                       THEN excluded.first_seen_block
                   ELSE addresses.first_seen_block
               END",
        )
        .bind(address.to_lowercase())
        .bind(first_seen_block)
        .execute(&self.pool)
        .await
        .context("upsert_address failed")?;
        Ok(())
    }

    pub async fn insert_transfer(&self, t: &Transfer) -> Result<()> {
        sqlx::query(
            "INSERT OR IGNORE INTO transfers
                (from_addr, to_addr, value, block_num, tx_hash, asset)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
        )
        .bind(t.from_addr.to_lowercase())
        .bind(t.to_addr.to_lowercase())
        .bind(t.value.as_deref())
        .bind(t.block_num)
        .bind(t.tx_hash.as_deref())
        .bind(t.asset.as_deref())
        .execute(&self.pool)
        .await
        .context("insert_transfer failed")?;
        Ok(())
    }

    pub async fn insert_transfers(&self, ts: &[Transfer]) -> Result<usize> {
        let mut tx = self.pool.begin().await?;
        let mut n = 0usize;
        for t in ts {
            let res = sqlx::query(
                "INSERT OR IGNORE INTO transfers
                    (from_addr, to_addr, value, block_num, tx_hash, asset)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            )
            .bind(t.from_addr.to_lowercase())
            .bind(t.to_addr.to_lowercase())
            .bind(t.value.as_deref())
            .bind(t.block_num)
            .bind(t.tx_hash.as_deref())
            .bind(t.asset.as_deref())
            .execute(&mut *tx)
            .await
            .context("batch insert_transfer failed")?;
            n += res.rows_affected() as usize;
        }
        tx.commit().await?;
        Ok(n)
    }

    pub async fn incoming_funders(&self, address: &str) -> Result<Vec<(String, i64)>> {
        let rows = sqlx::query(
            "SELECT from_addr, COALESCE(MIN(block_num), 0) AS first_block
             FROM transfers
             WHERE to_addr = ?1
             GROUP BY from_addr
             ORDER BY first_block ASC",
        )
        .bind(address.to_lowercase())
        .fetch_all(&self.pool)
        .await
        .context("incoming_funders query failed")?;

        Ok(rows
            .into_iter()
            .map(|r| {
                let from: String = r.get("from_addr");
                let block: i64 = r.get("first_block");
                (from, block)
            })
            .collect())
    }

    pub async fn known_addresses(&self) -> Result<Vec<String>> {
        let rows = sqlx::query("SELECT address FROM addresses")
            .fetch_all(&self.pool)
            .await
            .context("known_addresses query failed")?;
        Ok(rows.into_iter().map(|r| r.get::<String, _>(0)).collect())
    }
}
