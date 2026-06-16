use anyhow::{Context, Result};
use owo_colors::OwoColorize;
use reqwest::header::{ACCEPT, AUTHORIZATION};
use tabled::{
    Table, Tabled,
    settings::{Color, Style, object::Rows},
};

use crate::auth::get_token;
use crate::config::ConfigFile;
use crate::utils::{
    AdapterDirection, DetailRow, fetch_iflow_adapters, format_timestamp, summarise_adapters,
};

// ── Display structs ───────────────────────────────────────────────────────────

#[derive(Tabled)]
struct ConfigurationRow {
    #[tabled(rename = "Parameter Name")]
    key: String,
    #[tabled(rename = "Parameter Value")]
    value: String,
    #[tabled(rename = "Data Type")]
    data_type: String,
}

#[derive(Tabled)]
struct ResourceRow {
    #[tabled(rename = "Name")]
    name: String,
    #[tabled(rename = "Type")]
    resource_type: String,
}

// ── Fetch helpers ─────────────────────────────────────────────────────────────

async fn fetch_configurations(
    client: &reqwest::Client,
    base_url: &str,
    token: &str,
    id: &str,
    version: &str,
) -> Result<Vec<ConfigurationRow>> {
    let url = format!(
        "{}/api/v1/IntegrationDesigntimeArtifacts(Id='{}',Version='{}')/Configurations",
        base_url, id, version
    );

    let text = client
        .get(&url)
        .header(ACCEPT, "application/xml")
        .header(AUTHORIZATION, format!("Bearer {}", token))
        .send()
        .await
        .context("failed to fetch configurations")?
        .text()
        .await
        .context("failed to read configurations body")?;

    let doc = roxmltree::Document::parse(&text).context("failed to parse configurations XML")?;

    let rows = doc
        .descendants()
        .filter(|n| n.tag_name().name() == "entry")
        .filter_map(|entry| {
            let props = entry
                .descendants()
                .find(|n| n.tag_name().name() == "properties")?;

            let get = |name: &str| -> String {
                props
                    .children()
                    .find(|n| n.tag_name().name() == name)
                    .and_then(|n| n.text())
                    .unwrap_or("")
                    .trim()
                    .to_string()
            };

            Some(ConfigurationRow {
                key: get("ParameterKey"),
                value: get("ParameterValue"),
                data_type: get("DataType"),
            })
        })
        .collect();

    Ok(rows)
}

async fn fetch_resources(
    client: &reqwest::Client,
    base_url: &str,
    token: &str,
    id: &str,
    version: &str,
) -> Result<Vec<ResourceRow>> {
    let url = format!(
        "{}/api/v1/IntegrationDesigntimeArtifacts(Id='{}',Version='{}')/Resources",
        base_url, id, version
    );

    let text = client
        .get(&url)
        .header(ACCEPT, "application/xml")
        .header(AUTHORIZATION, format!("Bearer {}", token))
        .send()
        .await
        .context("failed to fetch resources")?
        .text()
        .await
        .context("failed to read resources body")?;

    let doc = roxmltree::Document::parse(&text).context("failed to parse resources XML")?;

    let rows = doc
        .descendants()
        .filter(|n| n.tag_name().name() == "entry")
        .filter_map(|entry| {
            let props = entry
                .descendants()
                .find(|n| n.tag_name().name() == "properties")?;

            let get = |name: &str| -> String {
                props
                    .children()
                    .find(|n| n.tag_name().name() == name)
                    .and_then(|n| n.text())
                    .unwrap_or("")
                    .trim()
                    .to_string()
            };

            Some(ResourceRow {
                name: get("Name"),
                resource_type: get("ResourceType"),
            })
        })
        .collect();

    Ok(rows)
}

// ── Section header helper ─────────────────────────────────────────────────────

fn print_section(label: &str, count: usize, pad: usize) {
    println!(
        "\n{}",
        format!("── {} ({}) {}", label, count, "─".repeat(pad))
            .bright_cyan()
            .bold()
    );
}

// ── Public entry point ────────────────────────────────────────────────────────

pub async fn handle(
    config: &ConfigFile,
    profile: &str,
    id: String,
    version: String,
    show_configurations: bool,
    show_resources: bool,
) -> Result<()> {
    let token = get_token(config, profile).await?;
    let client = reqwest::Client::new();
    let oauth = config.get_profile(profile)?;
    let base_url = oauth.url.trim_end_matches('/');

    // ── Main iflow detail ─────────────────────────────────────────────────────
    let url = format!(
        "{}/api/v1/IntegrationDesigntimeArtifacts(Id='{}',Version='{}')",
        base_url, id, version
    );

    let response = client
        .get(&url)
        .header(ACCEPT, "application/xml")
        .header(AUTHORIZATION, format!("Bearer {}", token))
        .send()
        .await
        .context("failed to fetch iflow details")?;

    if !response.status().is_success() {
        anyhow::bail!(
            "request failed with status: {} — check that the id and version are correct",
            response.status()
        );
    }

    let text = response
        .text()
        .await
        .context("failed to read response body")?;
    let doc = roxmltree::Document::parse(&text).context("failed to parse XML response")?;

    let props = doc
        .descendants()
        .find(|n| n.tag_name().name() == "properties")
        .context("missing <m:properties> in response")?;

    let get = |name: &str| -> String {
        props
            .children()
            .find(|n| n.tag_name().name() == name)
            .and_then(|n| n.text())
            .unwrap_or("")
            .trim()
            .to_string()
    };

    let iflow_id = get("Id");
    let iflow_version = get("Version");
    let description = get("Description");
    let sender = get("Sender");
    let receiver = get("Receiver");
    let comment = get("Comment");

    // Fire all optional requests concurrently alongside adapter fetch
    let (adapter_result, configs_result, resources_result) = tokio::join!(
        fetch_iflow_adapters(&client, base_url, &token, &iflow_id, &iflow_version),
        async {
            if show_configurations {
                fetch_configurations(&client, base_url, &token, &iflow_id, &iflow_version).await
            } else {
                Ok(vec![])
            }
        },
        async {
            if show_resources {
                fetch_resources(&client, base_url, &token, &iflow_id, &iflow_version).await
            } else {
                Ok(vec![])
            }
        }
    );

    let (sender_adapter, receiver_adapter) = match adapter_result {
        Ok(adapters) => {
            let s = summarise_adapters(
                adapters
                    .iter()
                    .filter(|a| a.direction == AdapterDirection::Sender)
                    .map(|a| a.component_type.as_str())
                    .collect(),
            );
            let r = summarise_adapters(
                adapters
                    .iter()
                    .filter(|a| a.direction == AdapterDirection::Receiver)
                    .map(|a| a.component_type.as_str())
                    .collect(),
            );
            (
                if s.is_empty() { "—".to_string() } else { s },
                if r.is_empty() { "—".to_string() } else { r },
            )
        }
        Err(_) => ("—".to_string(), "—".to_string()),
    };

    println!("[{}]", profile);

    let detail_rows = vec![
        DetailRow {
            field: "ID".into(),
            value: iflow_id.clone(),
        },
        DetailRow {
            field: "Name".into(),
            value: get("Name"),
        },
        DetailRow {
            field: "Version".into(),
            value: iflow_version.clone(),
        },
        DetailRow {
            field: "Package".into(),
            value: get("PackageId"),
        },
        DetailRow {
            field: "Description".into(),
            value: if description.is_empty() {
                "—".into()
            } else {
                description
            },
        },
        DetailRow {
            field: "Sender".into(),
            value: if sender.is_empty() {
                "—".into()
            } else {
                sender
            },
        },
        DetailRow {
            field: "Receiver".into(),
            value: if receiver.is_empty() {
                "—".into()
            } else {
                receiver
            },
        },
        DetailRow {
            field: "Sender Adapter".into(),
            value: sender_adapter,
        },
        DetailRow {
            field: "Receiver Adapter".into(),
            value: receiver_adapter,
        },
        DetailRow {
            field: "Created By".into(),
            value: get("CreatedBy"),
        },
        DetailRow {
            field: "Created At".into(),
            value: format_timestamp(&get("CreatedAt")),
        },
        DetailRow {
            field: "Modified By".into(),
            value: get("ModifiedBy"),
        },
        DetailRow {
            field: "Modified At".into(),
            value: format_timestamp(&get("ModifiedAt")),
        },
        DetailRow {
            field: "Comment".into(),
            value: if comment.is_empty() {
                "—".into()
            } else {
                comment
            },
        },
    ];

    let mut table = Table::new(&detail_rows);
    table.with(Style::rounded());
    table.modify(Rows::first(), Color::FG_BRIGHT_GREEN);
    println!("{table}");

    // ── Configurations ────────────────────────────────────────────────────────
    if show_configurations {
        let rows = configs_result.unwrap_or_default();
        print_section("Configurations", rows.len(), 40);
        if rows.is_empty() {
            println!("  No configurations found.");
        } else {
            let mut t = Table::new(&rows);
            t.with(Style::rounded());
            t.modify(Rows::first(), Color::FG_BRIGHT_GREEN);
            println!("{t}");
        }
    }

    // ── Resources ─────────────────────────────────────────────────────────────
    if show_resources {
        let rows = resources_result.unwrap_or_default();
        print_section("Resources", rows.len(), 46);
        if rows.is_empty() {
            println!("  No resources found.");
        } else {
            let mut t = Table::new(&rows);
            t.with(Style::rounded());
            t.modify(Rows::first(), Color::FG_BRIGHT_GREEN);
            println!("{t}");
        }
    }

    Ok(())
}
