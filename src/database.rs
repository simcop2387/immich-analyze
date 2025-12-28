use crate::error::ImageAnalysisError;
use serde::Serialize;
use tokio_postgres::Client as PgClient;
use uuid::Uuid;
use chrono::NaiveDate;

#[derive(Debug, Serialize)]
pub struct ImageAnalysisResult {
    pub description: String,
    pub asset_id: Uuid,
}

// Grab some data about the persons that are present, this way we can use them to describe the
// people in the image better.  Maybe someday also include the face data to provide to the VLM?
#[derive(Debug, Serialize)]
pub struct ImmichPersonResult {
    pub name: String,
    pub birth_date: Option<String>, // Right type?
    pub is_favorite: bool, // Useful?
}

// Any OCR that was found in the image, might be useful for helping smaller local models pull the
// data into the description to better summarize things if the local model can't OCR as well
#[derive(Debug, Serialize)]
pub struct ImmichAssetOCR {
    pub text: String,
}

#[derive(Debug, Serialize)]
pub struct ImmichAssetTags {
    pub value: String,
    pub color: Option<String>,
}

// Get a list of the albums this asset is in
#[derive(Debug, Serialize)]
pub struct ImmichAssetAlbum {
    pub name: String,
    pub description: String,
}

// Final bit to serialize into yaml or json? into the prompt
#[derive(Debug, Serialize)]
pub struct ImmichAssetMetadata {
    pub ocrs: Vec<ImmichAssetOCR>,
    pub tags: Vec<ImmichAssetTags>,
    pub albums: Vec<ImmichAssetAlbum>,
}

async fn fetch_albums_for_asset(
    client: &PgClient,
    asset_id: Uuid,
) -> Result<Vec<ImmichAssetAlbum>, ImageAnalysisError> {
    let query = "
        SELECT a.name, a.description
        FROM album_asset aa
        JOIN album a ON aa.\"albumId\" = a.id
        WHERE aa.\"assetId\" = $1
    ";
    let asset_id_str = asset_id.to_string();
    let rows = client.query(query, &[&asset_id_str]).await.map_err(|e| {
        eprintln!(
            "{}",
            rust_i18n::t!("database.error_fetching_albums", error = e.to_string())
        );
        ImageAnalysisError::DatabaseError {
            error: e.to_string(),
        }
    })?;

    let mut albums = Vec::new();
    for row in rows {
        let name: String = row.get("name");
        let description: String = row.get("description");
        albums.push(ImmichAssetAlbum { name, description });
    }
    Ok(albums)
}

/// Optional: Fetch person info (for future use with VLM/face recognition)
pub async fn fetch_persons_for_asset(
    client: &PgClient,
    asset_id: Uuid,
) -> Result<Vec<ImmichPersonResult>, ImageAnalysisError> {
    let query = "
        SELECT p.name, p.birthDate, p.isFavorite
        FROM asset_face af
        JOIN person p ON af.\"personId\" = p.id
        WHERE af.\"assetId\" = $1
    ";
    let asset_id_str = asset_id.to_string();
    let rows = client.query(query, &[&asset_id_str]).await.map_err(|e| {
        eprintln!(
            "{}",
            rust_i18n::t!("database.error_fetching_persons", error = e.to_string())
        );
        ImageAnalysisError::DatabaseError {
            error: e.to_string(),
        }
    })?;

    let mut persons = Vec::new();
    for row in rows {
        let name: String = row.get("name");
        let birth_date: Option<NaiveDate> = NaiveDate::parse_from_str(row.get("birthDate"), "%Y-%m-%d").ok();
        let is_favorite: bool = row.get("isFavorite");

        // locales for formatting? or is iso8601 going to be better no matter what anyway
        let birth_date_str = birth_date.map(|date| date.format("%Y-%m-%d").to_string());

        persons.push(ImmichPersonResult {
            name,
            birth_date: birth_date_str,
            is_favorite,
        });
    }
    Ok(persons)
}


/// Fetches all metadata (OCR, Tags, Albums) for a given asset ID.
pub async fn get_asset_metadata(
    client: &PgClient,
    asset_id: Uuid,
) -> Result<ImmichAssetMetadata, ImageAnalysisError> {
    // 1. Fetch OCR data
    let ocrs = fetch_ocr_for_asset(client, asset_id).await?;
    
    // 2. Fetch tags
    let tags = fetch_tags_for_asset(client, asset_id).await?;
    
    // 3. Fetch albums
    let albums = fetch_albums_for_asset(client, asset_id).await?;

    Ok(ImmichAssetMetadata {
        ocrs,
        tags,
        albums,
    })
}

/// Fetches all OCR entries for the given asset_id
async fn fetch_ocr_for_asset(
    client: &PgClient,
    asset_id: Uuid,
) -> Result<Vec<ImmichAssetOCR>, ImageAnalysisError> {
    let query = "
        SELECT text 
        FROM asset_ocr 
        WHERE \"assetId\" = $1
    ";
    let asset_id_str = asset_id.to_string();
    let rows = client.query(query, &[&asset_id_str]).await.map_err(|e| {
        eprintln!(
            "{}",
            rust_i18n::t!("database.error_fetching_ocr", error = e.to_string())
        );
        ImageAnalysisError::DatabaseError {
            error: e.to_string(),
        }
    })?;

    let mut ocrs = Vec::new();
    for row in rows {
        let text: String = row.get("text");
        ocrs.push(ImmichAssetOCR {
            text,
        });
    }
    Ok(ocrs)
}


/// Fetches all tags associated with the asset
async fn fetch_tags_for_asset(
    client: &PgClient,
    asset_id: Uuid,
) -> Result<Vec<ImmichAssetTags>, ImageAnalysisError> {
    let query = "
        SELECT t.value, t.color
        FROM tag_asset ta
        JOIN tag t ON ta.\"tagId\" = t.id
        WHERE ta.\"assetId\" = $1
    ";
    let asset_id_str = asset_id.to_string();
    let rows = client.query(query, &[&asset_id_str]).await.map_err(|e| {
        eprintln!(
            "{}",
            rust_i18n::t!("database.error_fetching_tags", error = e.to_string())
        );
        ImageAnalysisError::DatabaseError {
            error: e.to_string(),
        }
    })?;

    let mut tags = Vec::new();
    for row in rows {
        let value: String = row.get("value");
        let color: Option<String> = row.get("color");
        tags.push(ImmichAssetTags { value, color });
    }
    Ok(tags)
}


/// Check if asset already has description in database
pub async fn asset_has_description(
    client: &PgClient,
    asset_id: Uuid,
) -> Result<bool, ImageAnalysisError> {
    let query = "
        SELECT EXISTS (
            SELECT 1 FROM asset_exif 
            WHERE \"assetId\"::text = $1 
            AND description IS NOT NULL 
            AND description != ''
        )
    ";
    let asset_id_str = asset_id.to_string();
    match client.query_one(query, &[&asset_id_str]).await {
        Ok(row) => Ok(row.get(0)),
        Err(e) => {
            eprintln!(
                "{}",
                rust_i18n::t!("database.error_checking_description", error = e.to_string())
            );
            Err(ImageAnalysisError::DatabaseError {
                error: e.to_string(),
            })
        }
    }
}

/// Update or create asset description in database
pub async fn update_or_create_asset_description(
    client: &PgClient,
    asset_id: Uuid,
    description: &str,
) -> Result<(), ImageAnalysisError> {
    let safe_description = description.replace("'", "''");
    let safe_asset_id = asset_id.to_string();
    println!(
        "{}",
        rust_i18n::t!("database.updating_asset", asset_id = asset_id.to_string())
    );
    let preview: String = description.chars().take(100).collect();
    println!(
        "{}",
        rust_i18n::t!(
            "database.description_length",
            length = description.len().to_string(),
            preview = preview
        )
    );

    let update_query = format!(
        r#"
        UPDATE asset_exif 
        SET description = E'{}', 
            "updatedAt" = NOW(),
            "updateId" = immich_uuid_v7()
        WHERE "assetId" = '{}'
        "#,
        safe_description, safe_asset_id
    );
    match client.execute(&update_query, &[]).await {
        Ok(rows_affected) => {
            if rows_affected > 0 {
                println!(
                    "{}",
                    rust_i18n::t!("database.update_success", asset_id = asset_id.to_string())
                );
                return Ok(());
            }
        }
        Err(e) => {
            eprintln!(
                "{}\n{}",
                rust_i18n::t!(
                    "database.update_error",
                    asset_id = asset_id.to_string(),
                    error = e.to_string()
                ),
                rust_i18n::t!("database.sql_query_details", query = update_query)
            );
            return Err(ImageAnalysisError::DatabaseError {
                error: e.to_string(),
            });
        }
    }

    let asset_exists_query = format!(
        r#"
        SELECT EXISTS (
            SELECT 1 FROM asset 
            WHERE id = '{}'
        )
        "#,
        safe_asset_id
    );
    let asset_exists = match client.query_one(&asset_exists_query, &[]).await {
        Ok(row) => row.get::<_, bool>(0),
        Err(e) => {
            eprintln!(
                "{}",
                rust_i18n::t!(
                    "database.asset_existence_check_error",
                    error = e.to_string()
                )
            );
            return Err(ImageAnalysisError::DatabaseError {
                error: e.to_string(),
            });
        }
    };
    if !asset_exists {
        eprintln!(
            "{}",
            rust_i18n::t!(
                "database.asset_not_in_table",
                asset_id = asset_id.to_string()
            )
        );
        return Err(ImageAnalysisError::DatabaseError {
            error: format!(
                "{}",
                rust_i18n::t!(
                    "database.asset_not_found_error",
                    asset_id = asset_id.to_string()
                )
            ),
        });
    }

    let insert_query = format!(
        r#"
        INSERT INTO asset_exif (
            "assetId", description, "updatedAt", "updateId"
        ) VALUES (
            '{}', E'{}', NOW(), immich_uuid_v7()
        )
        ON CONFLICT ("assetId") DO UPDATE 
        SET description = EXCLUDED.description,
            "updatedAt" = NOW(),
            "updateId" = immich_uuid_v7()
        "#,
        safe_asset_id, safe_description
    );

    match client.execute(&insert_query, &[]).await {
        Ok(_) => {
            println!(
                "{}",
                rust_i18n::t!("database.insert_success", asset_id = asset_id.to_string())
            );
            Ok(())
        }
        Err(e) => {
            eprintln!(
                "{}\n{}",
                rust_i18n::t!(
                    "database.insert_error",
                    asset_id = asset_id.to_string(),
                    error = e.to_string()
                ),
                rust_i18n::t!("database.sql_query_details", query = insert_query)
            );
            Err(ImageAnalysisError::DatabaseError {
                error: e.to_string(),
            })
        }
    }
}

pub async fn check_database_connection(client: &PgClient) -> Result<bool, ImageAnalysisError> {
    let timeout_duration = std::time::Duration::from_secs(5);
    match tokio::time::timeout(timeout_duration, client.query("SELECT 1", &[])).await {
        Ok(Ok(_)) => {
            println!("{}", rust_i18n::t!("database.connection_success"));
            Ok(true)
        }
        Ok(Err(e)) => {
            eprintln!(
                "{}",
                rust_i18n::t!("error.database_query_failed", error = e.to_string())
            );
            Err(ImageAnalysisError::DatabaseError {
                error: format!(
                    "{}",
                    rust_i18n::t!("database.query_failed_error", error = e.to_string())
                ),
            })
        }
        Err(_) => {
            eprintln!("{}", rust_i18n::t!("error.database_timeout"));
            Err(ImageAnalysisError::DatabaseError {
                error: format!("{}", rust_i18n::t!("database.timeout_error")),
            })
        }
    }
}
