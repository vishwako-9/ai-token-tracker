use colored::Colorize;

use crate::models::{DailyRow, ModelPricing, SummaryRow, UsageRecord};

fn fmt_int(n: i64) -> String {
    let s = n.to_string();
    let mut out = String::new();
    for (i, c) in s.chars().rev().enumerate() {
        if i > 0 && i % 3 == 0 {
            out.push(',');
        }
        out.push(c);
    }
    out.chars().rev().collect()
}

fn fmt_cost(cost: f64) -> String {
    if cost == 0.0 {
        "0".to_string()
    } else if cost < 0.01 {
        format!("${:.4}", cost)
    } else {
        format!("${:.2}", cost)
    }
}

struct Col {
    header: String,
    rows: Vec<String>,
    align_right: bool,
}

fn render_table(title: &str, cols: Vec<Col>, header_color: &str) {
    let widths: Vec<usize> = cols
        .iter()
        .map(|c| {
            let max = c
                .rows
                .iter()
                .map(|r| r.chars().count())
                .max()
                .unwrap_or(0);
            max.max(c.header.chars().count())
        })
        .collect();

    let mut line = String::from("┌");
    for (i, w) in widths.iter().enumerate() {
        line.push_str(&"─".repeat(w + 2));
        line.push(if i + 1 < widths.len() { '┬' } else { '┐' });
    }

    let sep = {
        let mut s = String::from("├");
        for (i, w) in widths.iter().enumerate() {
            s.push_str(&"─".repeat(w + 2));
            s.push(if i + 1 < widths.len() { '┼' } else { '┤' });
        }
        s
    };

    println!();
    println!("{}", title.bold());
    println!("{line}");

    let header_line = build_row(&cols.iter().map(|c| c.header.clone()).collect::<Vec<_>>(), &widths, &vec![false; widths.len()]);
    match header_color {
        "cyan" => println!("{}", header_line.cyan()),
        "yellow" => println!("{}", header_line.yellow()),
        _ => println!("{header_line}"),
    }
    println!("{sep}");

    let n = cols.first().map(|c| c.rows.len()).unwrap_or(0);
    for r in 0..n {
        let row_vals: Vec<String> = cols.iter().map(|c| c.rows[r].clone()).collect();
        let aligns: Vec<bool> = cols.iter().map(|c| c.align_right).collect();
        println!("{}", build_row(&row_vals, &widths, &aligns));
    }

    println!("{}", line.replace(['┬', '┼'], "┴").replace(['┐', '┤'], "┘").replace(['┌', '├'], "└"));
}

fn build_row(vals: &[String], widths: &[usize], aligns: &[bool]) -> String {
    let mut s = String::from("│");
    for (i, (v, w)) in vals.iter().zip(widths.iter()).enumerate() {
        let pad = *w as i64 - v.chars().count() as i64;
        let right = aligns.get(i).copied().unwrap_or(false);
        let (before, after) = if right {
            (pad, 0)
        } else {
            (0, pad)
        };
        s.push(' ');
        s.push_str(&" ".repeat(before.max(0) as usize));
        s.push_str(v);
        s.push_str(&" ".repeat(after.max(0) as usize));
        s.push(' ');
        s.push('│');
    }
    s
}

pub fn print_summary(rows: &[SummaryRow]) {
    if rows.is_empty() {
        println!("{}", "No usage found in the selected period.".yellow());
        return;
    }

    let mut cols = vec![
        Col { header: "Provider".into(), rows: vec![], align_right: false },
        Col { header: "Model".into(), rows: vec![], align_right: false },
        Col { header: "Input".into(), rows: vec![], align_right: true },
        Col { header: "Output".into(), rows: vec![], align_right: true },
        Col { header: "Cache Read".into(), rows: vec![], align_right: true },
        Col { header: "Cache Write".into(), rows: vec![], align_right: true },
        Col { header: "Cost".into(), rows: vec![], align_right: true },
        Col { header: "Records".into(), rows: vec![], align_right: true },
    ];

    for r in rows {
        cols[0].rows.push(r.provider.clone());
        cols[1].rows.push(r.model.clone());
        cols[2].rows.push(fmt_int(r.total_input));
        cols[3].rows.push(fmt_int(r.total_output));
        cols[4].rows.push(fmt_int(r.total_cache_read));
        cols[5].rows.push(fmt_int(r.total_cache_write));
        cols[6].rows.push(fmt_cost(r.total_cost));
        cols[7].rows.push(fmt_int(r.record_count));
    }

    render_table("Usage Summary", cols, "cyan");
}

pub fn print_daily(rows: &[DailyRow], title: &str, show_all: bool) {
    let filtered: Vec<&DailyRow> = rows
        .iter()
        .filter(|r| show_all || r.total_input > 0 || r.total_output > 0)
        .collect();

    if filtered.is_empty() {
        println!("{}", "No usage found in the selected period.".yellow());
        return;
    }

    let mut cols = vec![
        Col { header: "Period".into(), rows: vec![], align_right: false },
        Col { header: "Models".into(), rows: vec![], align_right: false },
        Col { header: "Input".into(), rows: vec![], align_right: true },
        Col { header: "Output".into(), rows: vec![], align_right: true },
        Col { header: "Cost".into(), rows: vec![], align_right: true },
    ];

    for r in &filtered {
        cols[0].rows.push(r.date.clone());
        cols[1].rows.push(r.models.join(", "));
        cols[2].rows.push(fmt_int(r.total_input));
        cols[3].rows.push(fmt_int(r.total_output));
        cols[4].rows.push(fmt_cost(r.total_cost));
    }

    render_table(title, cols, "cyan");
}

pub fn print_detail(rows: &[UsageRecord]) {
    if rows.is_empty() {
        println!("{}", "No records found.".yellow());
        return;
    }
    let mut cols = vec![
        Col { header: "Recorded".into(), rows: vec![], align_right: false },
        Col { header: "Provider".into(), rows: vec![], align_right: false },
        Col { header: "Model".into(), rows: vec![], align_right: false },
        Col { header: "In".into(), rows: vec![], align_right: true },
        Col { header: "Out".into(), rows: vec![], align_right: true },
        Col { header: "Cost".into(), rows: vec![], align_right: true },
    ];
    for r in rows {
        cols[0].rows.push(r.recorded_at.chars().take(16).collect());
        cols[1].rows.push(r.provider.clone());
        cols[2].rows.push(r.model.clone());
        cols[3].rows.push(fmt_int(r.input_tokens));
        cols[4].rows.push(fmt_int(r.output_tokens));
        cols[5].rows.push(r.cost_usd.map(fmt_cost).unwrap_or_else(|| "-".into()));
    }
    render_table("Recent Records", cols, "cyan");
}

pub fn print_models(models: &[ModelPricing]) {
    if models.is_empty() {
        println!("{}", "No pricing data cached. Run `tokentracker update-pricing`.".yellow());
        return;
    }
    let mut cols = vec![
        Col { header: "Provider".into(), rows: vec![], align_right: false },
        Col { header: "Model".into(), rows: vec![], align_right: false },
        Col { header: "$/MTok In".into(), rows: vec![], align_right: true },
        Col { header: "$/MTok Out".into(), rows: vec![], align_right: true },
    ];
    for m in models {
        cols[0].rows.push(m.provider.clone());
        cols[1].rows.push(m.model.clone());
        cols[2].rows.push(format!("{:.4}", m.input_per_mtok));
        cols[3].rows.push(format!("{:.4}", m.output_per_mtok));
    }
    render_table("Model Pricing", cols, "yellow");
}

pub fn filter_daily_rows(rows: &[DailyRow], show_all: bool) -> Vec<DailyRow> {
    rows.iter()
        .filter(|r| show_all || r.total_input > 0 || r.total_output > 0)
        .cloned()
        .collect()
}

pub fn to_json(rows: &[UsageRecord]) -> anyhow::Result<String> {
    Ok(serde_json::to_string_pretty(rows)?)
}

pub fn to_csv(rows: &[UsageRecord]) -> String {
    let mut out = String::from("recorded_at,provider,model,input_tokens,output_tokens,cache_read_tokens,cache_write_tokens,reasoning_tokens,cost_usd,session_id\n");
    for r in rows {
        out.push_str(&format!(
            "{},{},{},{},{},{},{},{},{},{}\n",
            r.recorded_at,
            r.provider,
            r.model,
            r.input_tokens,
            r.output_tokens,
            r.cache_read_tokens,
            r.cache_write_tokens,
            r.reasoning_tokens,
            r.cost_usd.unwrap_or(0.0),
            r.session_id.clone().unwrap_or_default(),
        ));
    }
    out
}