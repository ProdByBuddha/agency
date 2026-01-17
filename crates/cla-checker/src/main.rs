use anyhow::{Context, Result};
use serde::Deserialize;
use std::env;
use std::fs;

#[derive(Deserialize, Debug)]
struct EventPayload {
    pull_request: Option<PullRequestDetail>,
    issue: Option<IssueDetail>,
}

#[derive(Deserialize, Debug)]
struct PullRequestDetail {
    number: u64,
    user: User,
}

#[derive(Deserialize, Debug)]
struct IssueDetail {
    number: u64,
    user: User, // In issue_comment, this is the issue author (PR author)
}

#[derive(Deserialize, Debug)]
struct User {
    login: String,
}

#[derive(Deserialize, Debug)]
struct Comment {
    user: User,
    body: String,
}

#[tokio::main]
async fn main() -> Result<()> {
    let token = env::var("GITHUB_TOKEN").context("GITHUB_TOKEN not set")?;
    let repo = env::var("GITHUB_REPOSITORY").context("GITHUB_REPOSITORY not set")?;
    let event_path = env::var("GITHUB_EVENT_PATH").context("GITHUB_EVENT_PATH not set")?;

    let event_data = fs::read_to_string(&event_path).context("Failed to read event file")?;
    let event: EventPayload = serde_json::from_str(&event_data).context("Failed to parse event JSON")?;

    // Determine PR number and Author
    // Note: For 'issue_comment' on a PR, the 'issue' field is populated.
    // For 'pull_request', the 'pull_request' field is populated.
    let (pr_number, author) = if let Some(pr) = event.pull_request {
        (pr.number, pr.user.login)
    } else if let Some(issue) = event.issue {
        (issue.number, issue.user.login)
    } else {
        println!("Not a Pull Request or Issue Comment event. Skipping.");
        return Ok(());
    };

    println!("Checking CLA for PR #{} by {}", pr_number, author);

    // Bypass for the repository owner (simplifies testing and self-merges)
    // You can customize this list or remove it.
    let owners = "ProdByBuddha";
    if owners.contains(&author.as_str()) {
         println!("Author '{}' is an owner. CLA check bypassed.", author);
         return Ok(());
    }

    let client = reqwest::Client::new();
    let url = format!("https://api.github.com/repos/{}/issues/{}/comments", repo, pr_number);

    let comments: Vec<Comment> = client
        .get(&url)
        .header("User-Agent", "cla-checker")
        .header("Authorization", format!("Bearer {}", token))
        .header("Accept", "application/vnd.github.v3+json")
        .send()
        .await?
        .json()
        .await?;

    let agreement_text = "I have read and agree to the CLA";
    let mut signed = false;

    for comment in comments {
        if comment.user.login == author && comment.body.contains(agreement_text) {
            signed = true;
            break;
        }
    }

    if signed {
        println!("✅ CLA signed by {}.", author);
        Ok(())
    } else {
        println!("❌ CLA not signed by {}.", author);
        println!("Please post a comment with the exact text:");
        println!("'{}'", agreement_text);
        std::process::exit(1);
    }
}
