use crate::error::ImageAnalysisError;
use serde::Serialize;
use tokio_postgres::Client as PgClient;
use uuid::Uuid;

#[derive(Debug, Serialize)]
pub struct ImageAnalysisResult {
    pub description: String,
    pub asset_id: Uuid,
}

// Grab some data about the persons that are present, this way we can use them to describe the
// people in the image better.
#[derive(Debug, Serialize)]
pub struct ImmichPersonResult {
    pub name: String,
    pub bounding_box: Option<BoundingBox>,
}

#[derive(Debug, Serialize)]
pub struct BoundingBox {
    pub x1: f64, // normalized 0.0–1.0
    pub y1: f64,
    pub x2: f64,
    pub y2: f64,
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

#[derive(Debug, Serialize)]
pub struct ImmichExternalFileInfo {
    pub original_filename: String,
    pub original_path: String,
}

#[derive(Debug, Serialize)]
pub struct ImmichAssetLocation {
    pub latitude: Option<f64>,
    pub longitude: Option<f64>,
    pub city: Option<String>,
    pub state: Option<String>,
    pub country: Option<String>,
}

// Final bit to serialize into yaml or json? into the prompt
#[derive(Debug, Serialize)]
pub struct ImmichAssetMetadata {
    pub ocrs: Vec<ImmichAssetOCR>,
    pub tags: Vec<ImmichAssetTags>,
    pub albums: Vec<ImmichAssetAlbum>,
    pub people: Vec<ImmichPersonResult>,
    pub location: Option<ImmichAssetLocation>,
    pub external_file: Option<ImmichExternalFileInfo>,
    /// The AI-generated description from the previous run, if any.
    pub previous_ai_description: String,
}

async fn fetch_albums_for_asset(
    client: &PgClient,
    asset_id: Uuid,
) -> Result<Vec<ImmichAssetAlbum>, ImageAnalysisError> {
    let query = "
        SELECT a.\"albumName\" AS name, a.description
        FROM album_asset aa
        JOIN album a ON aa.\"albumId\" = a.id
        WHERE aa.\"assetId\"::text = $1
    ";
    let asset_id_str = asset_id.to_string();
    let rows = client.query(query, &[&asset_id_str]).await.map_err(|e| {
        eprintln!(
            "{} {}",
            rust_i18n::t!("database.error_fetching_albums", error = e.to_string()),
            e.to_string()
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
        SELECT p.name,
               af.\"imageWidth\", af.\"imageHeight\",
               af.\"boundingBoxX1\", af.\"boundingBoxY1\",
               af.\"boundingBoxX2\", af.\"boundingBoxY2\"
        FROM asset_face af
        JOIN person p ON af.\"personId\" = p.id
        WHERE af.\"assetId\"::text = $1
          AND af.\"deletedAt\" IS NULL
          AND p.name IS NOT NULL
          AND p.name != ''
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
        let image_width: i32 = row.get("imageWidth");
        let image_height: i32 = row.get("imageHeight");
        let x1: i32 = row.get("boundingBoxX1");
        let y1: i32 = row.get("boundingBoxY1");
        let x2: i32 = row.get("boundingBoxX2");
        let y2: i32 = row.get("boundingBoxY2");

        let bounding_box = if image_width > 0 && image_height > 0 {
            Some(BoundingBox {
                x1: x1 as f64 / image_width as f64,
                y1: y1 as f64 / image_height as f64,
                x2: x2 as f64 / image_width as f64,
                y2: y2 as f64 / image_height as f64,
            })
        } else {
            None
        };

        persons.push(ImmichPersonResult { name, bounding_box });
    }
    Ok(persons)
}

/// Fetch location metadata (GPS + city/state/country) for an asset
async fn fetch_location_for_asset(
    client: &PgClient,
    asset_id: Uuid,
) -> Result<Option<ImmichAssetLocation>, ImageAnalysisError> {
    let query = "
        SELECT latitude, longitude, city, state, country
        FROM asset_exif
        WHERE \"assetId\"::text = $1
    ";
    let asset_id_str = asset_id.to_string();
    match client.query_opt(query, &[&asset_id_str]).await {
        Ok(Some(row)) => {
            let latitude: Option<f64> = row.get("latitude");
            let longitude: Option<f64> = row.get("longitude");
            let city: Option<String> = row.get("city");
            let state: Option<String> = row.get("state");
            let country: Option<String> = row.get("country");
            // Return None if all fields are NULL
            if latitude.is_none()
                && longitude.is_none()
                && city.is_none()
                && state.is_none()
                && country.is_none()
            {
                Ok(None)
            } else {
                Ok(Some(ImmichAssetLocation {
                    latitude,
                    longitude,
                    city,
                    state,
                    country,
                }))
            }
        }
        Ok(None) => Ok(None),
        Err(e) => {
            eprintln!(
                "{}",
                rust_i18n::t!("database.error_fetching_location", error = e.to_string())
            );
            Err(ImageAnalysisError::DatabaseError {
                error: e.to_string(),
            })
        }
    }
}

/// Returns originalFilename + originalPath for external assets, or None for non-external assets.
async fn fetch_external_file_info(
    client: &PgClient,
    asset_id: Uuid,
) -> Result<Option<ImmichExternalFileInfo>, ImageAnalysisError> {
    let query = "
        SELECT \"originalFileName\", \"originalPath\"
        FROM asset
        WHERE id::text = $1
          AND \"isExternal\" = true
    ";
    let asset_id_str = asset_id.to_string();
    match client.query_opt(query, &[&asset_id_str]).await {
        Ok(Some(row)) => Ok(Some(ImmichExternalFileInfo {
            original_filename: row.get("originalFileName"),
            original_path: row.get("originalPath"),
        })),
        Ok(None) => Ok(None),
        Err(e) => {
            eprintln!(
                "{}",
                rust_i18n::t!("database.error_fetching_external_file", error = e.to_string())
            );
            Err(ImageAnalysisError::DatabaseError {
                error: e.to_string(),
            })
        }
    }
}

/// Fetches all metadata (OCR, Tags, Albums) for a given asset ID.
pub async fn get_asset_metadata(
    client: &PgClient,
    asset_id: Uuid,
) -> Result<ImmichAssetMetadata, ImageAnalysisError> {
    eprintln!("[debug] get_asset_metadata: step 1 (ocr) for {}", asset_id);
    let ocrs = fetch_ocr_for_asset(client, asset_id).await?;

    eprintln!("[debug] get_asset_metadata: step 2 (tags) for {}", asset_id);
    let tags = fetch_tags_for_asset(client, asset_id).await?;

    eprintln!("[debug] get_asset_metadata: step 3 (albums) for {}", asset_id);
    let albums = fetch_albums_for_asset(client, asset_id).await?;

    eprintln!("[debug] get_asset_metadata: step 4 (people) for {}", asset_id);
    let people = fetch_persons_for_asset(client, asset_id).await?;

    eprintln!("[debug] get_asset_metadata: step 5 (location) for {}", asset_id);
    let location = fetch_location_for_asset(client, asset_id).await?;

    eprintln!("[debug] get_asset_metadata: step 6 (external_file) for {}", asset_id);
    let external_file = fetch_external_file_info(client, asset_id).await?;

    eprintln!("[debug] get_asset_metadata: all steps done for {}", asset_id);

    Ok(ImmichAssetMetadata {
        ocrs,
        tags,
        albums,
        people,
        location,
        external_file,
        previous_ai_description: String::new(),
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
        WHERE \"assetId\"::text = $1
    ";
    let asset_id_str = asset_id.to_string();
    let rows = client.query(query, &[&asset_id_str]).await.map_err(|e| {
        eprintln!(
            "{} {}",
            rust_i18n::t!("database.error_fetching_ocr", error = e.to_string()),
            e.to_string()
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
        WHERE ta.\"assetId\"::text = $1
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


/// Fetch the current description for an asset, if any
pub async fn get_existing_description(
    client: &PgClient,
    asset_id: Uuid,
) -> Result<Option<String>, ImageAnalysisError> {
    let query = "
        SELECT description FROM asset_exif
        WHERE \"assetId\"::text = $1
        AND description IS NOT NULL
        AND description != ''
    ";
    let asset_id_str = asset_id.to_string();
    match client.query_opt(query, &[&asset_id_str]).await {
        Ok(Some(row)) => Ok(Some(row.get("description"))),
        Ok(None) => Ok(None),
        Err(e) => {
            eprintln!(
                "[debug] get_existing_description error — is_closed={} full={:?}",
                e.is_closed(),
                e
            );
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
