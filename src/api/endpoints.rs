use serde::Deserialize;

use crate::error::Result;
use crate::post::Post;
use super::client::RedditClient;
use super::types::{Listing, Thing};

/// Response from /api/v1/me/friends
#[derive(Debug, Deserialize)]
pub struct FriendsListResponse {
    pub data: FriendsListData,
}

#[derive(Debug, Deserialize)]
pub struct FriendsListData {
    pub children: Vec<Friend>,
}

#[derive(Debug, Deserialize)]
pub struct Friend {
    pub name: String,
    #[serde(default)]
    pub date: Option<f64>,
    #[serde(default)]
    pub id: Option<String>,
}

/// A subreddit from /subreddits/mine/subscriber
#[derive(Debug, Deserialize)]
pub struct Subreddit {
    pub display_name: String,
    #[serde(default)]
    pub url: Option<String>,
    #[serde(default)]
    pub subreddit_type: Option<String>,
}

/// Fetch the authenticated user's friends list
pub async fn get_friends(client: &RedditClient) -> Result<Vec<Friend>> {
    let resp: FriendsListResponse = client
        .get_json("/api/v1/me/friends", &[("limit", "100")])
        .await?;
    Ok(resp.data.children)
}

/// Fetch subreddits the user subscribes to (paginated)
pub async fn get_subscriptions(
    client: &RedditClient,
    after: Option<&str>,
) -> Result<(Vec<Subreddit>, Option<String>)> {
    let mut params = vec![("limit", "100")];
    let after_owned;
    if let Some(a) = after {
        after_owned = a.to_string();
        params.push(("after", &after_owned));
    }
    let listing: Listing<Subreddit> = client
        .get_json("/subreddits/mine/subscriber", &params)
        .await?;
    let subs: Vec<Subreddit> = listing.data.children.into_iter().map(|t| t.data).collect();
    let next = listing.data.after;
    Ok((subs, next))
}

/// Fetch posts from a user's profile (submitted)
pub async fn get_user_posts(
    client: &RedditClient,
    username: &str,
    after: Option<&str>,
    limit: u32,
) -> Result<(Vec<Post>, Option<String>)> {
    let limit_str = limit.to_string();
    let mut params = vec![("limit", limit_str.as_str()), ("sort", "new"), ("raw_json", "1")];
    let after_owned;
    if let Some(a) = after {
        after_owned = a.to_string();
        params.push(("after", &after_owned));
    }
    let listing: Listing<Post> = client
        .get_json(&format!("/user/{}/submitted", username), &params)
        .await?;
    let posts: Vec<Post> = listing.data.children.into_iter().map(|t| t.data).collect();
    let next = listing.data.after;
    Ok((posts, next))
}

/// Fetch a user's saved posts
pub async fn get_saved(
    client: &RedditClient,
    username: &str,
    after: Option<&str>,
    limit: u32,
) -> Result<(Vec<Post>, Option<String>)> {
    let limit_str = limit.to_string();
    let mut params = vec![("limit", limit_str.as_str()), ("raw_json", "1")];
    let after_owned;
    if let Some(a) = after {
        after_owned = a.to_string();
        params.push(("after", &after_owned));
    }
    let listing: Listing<Post> = client
        .get_json(&format!("/user/{}/saved", username), &params)
        .await?;
    let posts: Vec<Post> = listing.data.children.into_iter().map(|t| t.data).collect();
    let next = listing.data.after;
    Ok((posts, next))
}

/// Fetch new posts from a subreddit
pub async fn get_subreddit_posts(
    client: &RedditClient,
    subreddit: &str,
    after: Option<&str>,
    limit: u32,
) -> Result<(Vec<Post>, Option<String>)> {
    let limit_str = limit.to_string();
    let mut params = vec![("limit", limit_str.as_str()), ("raw_json", "1")];
    let after_owned;
    if let Some(a) = after {
        after_owned = a.to_string();
        params.push(("after", &after_owned));
    }
    let listing: Listing<Post> = client
        .get_json(&format!("/r/{}/new", subreddit), &params)
        .await?;
    let posts: Vec<Post> = listing.data.children.into_iter().map(|t| t.data).collect();
    let next = listing.data.after;
    Ok((posts, next))
}

/// Fetch comments for a post
pub async fn get_post_comments(
    client: &RedditClient,
    subreddit: &str,
    post_id: &str,
) -> Result<serde_json::Value> {
    let resp: serde_json::Value = client
        .get_json(
            &format!("/r/{}/comments/{}", subreddit, post_id),
            &[("raw_json", "1")],
        )
        .await?;
    Ok(resp)
}
