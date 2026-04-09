use serde::Deserialize;

/// Response from /api/v1/me
#[allow(dead_code)]
#[derive(Debug, Deserialize)]
pub struct MeResponse {
    pub name: String,
    pub id: String,
}

/// Reddit's standard listing pagination wrapper
#[derive(Debug, Deserialize)]
pub struct Listing<T> {
    pub data: ListingData<T>,
}

#[allow(dead_code)]
#[derive(Debug, Deserialize)]
pub struct ListingData<T> {
    pub children: Vec<Thing<T>>,
    pub after: Option<String>,
    pub before: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct Thing<T> {
    pub kind: String,
    pub data: T,
}
