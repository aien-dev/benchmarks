use crate::models::BenchmarkData;

pub fn generate_memory_chart(data: &BenchmarkData) -> String {
    let mut bars = String::new();
    let max_val = 3800.0f64;
    let chart_w = 600.0f64;
    let start_y = 120.0f64;
    let row_h = 58.0f64;

    for (i, m) in data.memory_rss.iter().enumerate() {
        let y = start_y + (i as f64) * row_h;
        let bar_len = ((m.rss_mb / max_val) * chart_w).max(4.0);
        let color = if m.rss_mb > 500.0 {
            "#d7755d"
        } else if m.rss_mb > 40.0 {
            "#d3a85b"
        } else {
            "#7fb8a6"
        };

        let badge = if m.reduction_pct > 0.0 {
            format!("(-{:.1}%)", m.reduction_pct)
        } else {
            "(Baseline)".to_string()
        };

        bars.push_str(&format!(
            r#"
    <g class="bar-row">
      <text x="30" y="{label_y}" class="label-service">{service}</text>
      <text x="30" y="{desc_y}" class="label-role">{role} [{arch}]</text>
      <rect x="280" y="{rect_y}" width="{w}" height="26" rx="4" fill="{color}" />
      <text x="{val_x}" y="{val_y}" class="label-val">{rss:.2} MB <tspan class="badge">{badge}</tspan></text>
    </g>"#,
            label_y = y,
            desc_y = y + 15.0,
            rect_y = y - 16.0,
            w = bar_len,
            color = color,
            val_x = 290.0 + bar_len + 12.0,
            val_y = y + 2.0,
            service = m.service,
            role = m.role,
            arch = m.architecture,
            rss = m.rss_mb,
            badge = badge,
        ));
    }

    format!(
        r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 1100 520" width="100%" height="100%">
  <defs>
    <style>
      .bg {{ fill: #18111f; }}
      .title {{ font-family: ui-sans-serif, system-ui, sans-serif; font-size: 20px; font-weight: 700; fill: #fff8ee; }}
      .subtitle {{ font-family: ui-sans-serif, system-ui, sans-serif; font-size: 13px; fill: #bca9c2; }}
      .label-service {{ font-family: ui-monospace, monospace; font-size: 14px; font-weight: 700; fill: #ded3df; }}
      .label-role {{ font-family: ui-sans-serif, system-ui, sans-serif; font-size: 11px; fill: #8eac78; }}
      .label-val {{ font-family: ui-monospace, monospace; font-size: 13px; font-weight: 700; fill: #fff8ee; }}
      .badge {{ font-size: 11px; font-weight: 600; fill: #ef8b67; }}
      .grid {{ stroke: rgba(222, 211, 223, 0.12); stroke-width: 1; }}
    </style>
  </defs>
  <rect width="100%" height="100%" rx="12" class="bg" />
  <text x="30" y="48" class="title">Memory Resident Set Size (RSS): Native Rust vs Python Stacks</text>
  <text x="30" y="74" class="subtitle">Measured on NVIDIA DGX Spark (Grace Blackwell GB10, aarch64) via /proc/[pid]/status VmRSS</text>
  <line x1="280" y1="95" x2="280" y2="480" class="grid" />
  <line x1="580" y1="95" x2="580" y2="480" class="grid" />
  <line x1="880" y1="95" x2="880" y2="480" class="grid" />
{bars}
</svg>"#,
        bars = bars
    )
}

pub fn generate_latency_chart(data: &BenchmarkData) -> String {
    let mut bars = String::new();
    let max_val = 45.0f64;
    let chart_w = 560.0f64;
    let start_y = 120.0f64;
    let row_h = 68.0f64;

    for (i, m) in data.latency_concurrency.iter().enumerate() {
        let y = start_y + (i as f64) * row_h;
        let bar_len = ((m.p50_ms / max_val) * chart_w).max(6.0);
        let color = if m.p50_ms > 20.0 {
            "#d7755d"
        } else {
            "#7fb8a6"
        };

        bars.push_str(&format!(
            r#"
    <g class="bar-row">
      <text x="30" y="{label_y}" class="label-service">{service} {endpoint}</text>
      <text x="30" y="{desc_y}" class="label-role">{desc} [{engine}]</text>
      <rect x="360" y="{rect_y}" width="{w}" height="28" rx="4" fill="{color}" />
      <text x="{val_x}" y="{val_y}" class="label-val">{p50:.2} ms <tspan class="throughput">({rps:.1} req/s)</tspan></text>
    </g>"#,
            label_y = y,
            desc_y = y + 16.0,
            rect_y = y - 16.0,
            w = bar_len,
            color = color,
            val_x = 370.0 + bar_len + 12.0,
            val_y = y + 3.0,
            service = m.service,
            endpoint = m.endpoint,
            desc = m.description,
            engine = m.engine,
            p50 = m.p50_ms,
            rps = m.requests_per_sec,
        ));
    }

    format!(
        r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 1100 500" width="100%" height="100%">
  <defs>
    <style>
      .bg {{ fill: #18111f; }}
      .title {{ font-family: ui-sans-serif, system-ui, sans-serif; font-size: 20px; font-weight: 700; fill: #fff8ee; }}
      .subtitle {{ font-family: ui-sans-serif, system-ui, sans-serif; font-size: 13px; fill: #bca9c2; }}
      .label-service {{ font-family: ui-monospace, monospace; font-size: 14px; font-weight: 700; fill: #ded3df; }}
      .label-role {{ font-family: ui-sans-serif, system-ui, sans-serif; font-size: 11px; fill: #8eac78; }}
      .label-val {{ font-family: ui-monospace, monospace; font-size: 13px; font-weight: 700; fill: #fff8ee; }}
      .throughput {{ font-size: 12px; font-weight: 600; fill: #ef8b67; }}
      .grid {{ stroke: rgba(222, 211, 223, 0.12); stroke-width: 1; }}
    </style>
  </defs>
  <rect width="100%" height="100%" rx="12" class="bg" />
  <text x="30" y="48" class="title">HTTP Gateway Latency (p50 TTFB) and Throughput</text>
  <text x="30" y="74" class="subtitle">Localhost multi-threaded sweep: concurrency=10, 500 requests per endpoint on Grace Blackwell</text>
  <line x1="360" y1="95" x2="360" y2="470" class="grid" />
  <line x1="640" y1="95" x2="640" y2="470" class="grid" />
  <line x1="920" y1="95" x2="920" y2="470" class="grid" />
{bars}
</svg>"#,
        bars = bars
    )
}

pub fn generate_like_for_like_chart(data: &BenchmarkData) -> String {
    let mut rows = String::new();
    let max_val = 2.0f64; // Scale for latency in ms
    let chart_w = 400.0f64;
    let start_y = 120.0f64;
    let row_h = 100.0f64;

    for (i, m) in data.like_for_like.iter().enumerate() {
        let y = start_y + (i as f64) * row_h;
        let rust_w = ((m.rust_p50_ms / max_val) * chart_w).max(4.0);
        let py_w = ((m.python_p50_ms / max_val) * chart_w).max(4.0);
        let rust_color = "#7fb8a6";
        let py_color = "#d7755d";

        rows.push_str(&format!(
            r##"
    <g class="lfl-row">
      <text x="30" y="{label_y}" class="label-service">{category}: {workload}</text>
      <!-- Rust Bar -->
      <text x="50" y="{rust_y}" class="label-lang-rust">Rust Native:</text>
      <rect x="180" y="{rust_rect_y}" width="{rust_w}" height="18" rx="3" fill="{rust_color}" />
      <text x="{rust_val_x}" y="{rust_y}" class="label-val">{rust_p50:.2} ms ({rust_rps:.0} req/s)</text>

      <!-- Python Bar -->
      <text x="50" y="{py_y}" class="label-lang-py">CPython 3.12:</text>
      <rect x="180" y="{py_rect_y}" width="{py_w}" height="18" rx="3" fill="{py_color}" />
      <text x="{py_val_x}" y="{py_y}" class="label-val">{py_p50:.2} ms ({py_rps:.0} req/s)</text>

      <!-- Badge -->
      <text x="820" y="{badge_y}" class="badge-speedup">{speedup:.1}x Faster</text>
      <text x="820" y="{badge_mem_y}" class="badge-mem">(-{mem_pct:.1}% RAM)</text>
    </g>"##,
            label_y = y,
            rust_y = y + 24.0,
            rust_rect_y = y + 10.0,
            rust_w = rust_w,
            rust_val_x = 190.0 + rust_w + 10.0,
            rust_p50 = m.rust_p50_ms,
            rust_rps = m.rust_throughput_req_s,
            py_y = y + 48.0,
            py_rect_y = y + 34.0,
            py_w = py_w,
            py_val_x = 190.0 + py_w + 10.0,
            py_p50 = m.python_p50_ms,
            py_rps = m.python_throughput_req_s,
            badge_y = y + 26.0,
            badge_mem_y = y + 46.0,
            category = m.category,
            workload = m.workload,
            speedup = m.speedup_factor,
            mem_pct = m.memory_reduction_pct,
        ));
    }

    let svg_h = (start_y + (data.like_for_like.len() as f64) * row_h + 40.0) as u32;

    format!(
        r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 1100 {svg_h}" width="100%" height="100%">
  <defs>
    <style>
      .bg {{ fill: #18111f; }}
      .title {{ font-family: ui-sans-serif, system-ui, sans-serif; font-size: 20px; font-weight: 700; fill: #fff8ee; }}
      .subtitle {{ font-family: ui-sans-serif, system-ui, sans-serif; font-size: 13px; fill: #bca9c2; }}
      .label-service {{ font-family: ui-monospace, monospace; font-size: 13px; font-weight: 700; fill: #ded3df; }}
      .label-lang-rust {{ font-family: ui-monospace, monospace; font-size: 11px; font-weight: 600; fill: #7fb8a6; }}
      .label-lang-py {{ font-family: ui-monospace, monospace; font-size: 11px; font-weight: 600; fill: #d7755d; }}
      .label-val {{ font-family: ui-monospace, monospace; font-size: 12px; font-weight: 600; fill: #fff8ee; }}
      .badge-speedup {{ font-family: ui-sans-serif, system-ui, sans-serif; font-size: 15px; font-weight: 800; fill: #7fb8a6; }}
      .badge-mem {{ font-family: ui-sans-serif, system-ui, sans-serif; font-size: 12px; font-weight: 700; fill: #ef8b67; }}
    </style>
  </defs>
  <rect width="100%" height="100%" rx="12" class="bg" />
  <text x="30" y="48" class="title">Like-for-Like Microservice Benchmark: Rust Native vs CPython 3.12</text>
  <text x="30" y="74" class="subtitle">Identical payloads, in-memory SQLite tables, and vector workloads tested concurrently on Grace Blackwell</text>
{rows}
</svg>"#,
        svg_h = svg_h,
        rows = rows
    )
}
#[allow(dead_code)]
pub fn generate_pressure_chart(data: &BenchmarkData) -> String {
    let mut rows = String::new();
    let start_y = 120.0f64;
    let row_h = 44.0f64;
    let chart_w = 450.0f64;
    let max_ttft = 45.0f64;

    for (i, c) in data.concurrency_pressure.iter().enumerate() {
        let y = start_y + (i as f64) * row_h;
        let ttft_bar = ((c.aien_ttft_p50_ms / max_ttft) * chart_w).max(4.0);
        let itl_bar = ((c.aien_itl_p50_ms / max_ttft) * chart_w).max(4.0);

        rows.push_str(&format!(
            r##"
    <g class="tier-row">
      <text x="30" y="{y}" class="label-concurrency">Concurrency = {concurrency}</text>
      <rect x="220" y="{rect_ttft_y}" width="{ttft_w}" height="14" rx="3" fill="#7fb8a6" />
      <rect x="220" y="{rect_itl_y}" width="{itl_w}" height="14" rx="3" fill="#6ba4d9" />
      <text x="{ttft_val_x}" y="{y_ttft_text}" class="label-stat">TTFT {ttft:.1}ms</text>
      <text x="{itl_val_x}" y="{y_itl_text}" class="label-stat">ITL {itl:.1}ms</text>
      <text x="760" y="{y}" class="badge-tps">{tps:.0} tok/s</text>
      <text x="920" y="{y}" class="badge-power">{power:.1}W ({joules:.4} J/tok)</text>
    </g>"##,
            y = y,
            concurrency = c.concurrency,
            rect_ttft_y = y - 14.0,
            rect_itl_y = y + 2.0,
            ttft_w = ttft_bar,
            itl_w = itl_bar,
            ttft_val_x = 230.0 + ttft_bar,
            itl_val_x = 230.0 + itl_bar,
            y_ttft_text = y - 3.0,
            y_itl_text = y + 13.0,
            ttft = c.aien_ttft_p50_ms,
            itl = c.aien_itl_p50_ms,
            tps = c.tokens_per_sec,
            power = c.power_watts,
            joules = c.joules_per_token,
        ));
    }

    let svg_h = (start_y + (data.concurrency_pressure.len() as f64) * row_h + 30.0) as u32;

    format!(
        r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 1100 {svg_h}" width="100%" height="100%">
  <defs>
    <style>
      .bg {{ fill: #18111f; }}
      .title {{ font-family: ui-sans-serif, system-ui, sans-serif; font-size: 20px; font-weight: 700; fill: #fff8ee; }}
      .subtitle {{ font-family: ui-sans-serif, system-ui, sans-serif; font-size: 13px; fill: #bca9c2; }}
      .label-concurrency {{ font-family: ui-monospace, monospace; font-size: 13px; font-weight: 700; fill: #ded3df; }}
      .label-stat {{ font-family: ui-monospace, monospace; font-size: 11px; font-weight: 600; fill: #fff8ee; }}
      .badge-tps {{ font-family: ui-monospace, monospace; font-size: 13px; font-weight: 700; fill: #7fb8a6; }}
      .badge-power {{ font-family: ui-monospace, monospace; font-size: 11px; font-weight: 600; fill: #d3a85b; }}
    </style>
  </defs>
  <rect width="100%" height="100%" rx="12" class="bg" />
  <text x="30" y="48" class="title">Empirical Concurrency Pressure Sweep (C=1 to C=256)</text>
  <text x="30" y="74" class="subtitle">AIEN Continuous Batching Scheduler + Paged Unified Memory KV Pool on NVIDIA Grace Blackwell GB10</text>
{rows}
</svg>"#,
        svg_h = svg_h,
        rows = rows
    )
}
#[allow(dead_code)]
pub fn generate_multi_model_chart(data: &BenchmarkData) -> String {
    let mut rows = String::new();
    let start_y = 120.0f64;
    let row_h = 58.0f64;
    let chart_w = 400.0f64;
    let max_ttft = 25.0f64;

    for (i, m) in data.multi_model_breadth.iter().enumerate() {
        let y = start_y + (i as f64) * row_h;
        let ttft_bar = ((m.ttft_p50_ms / max_ttft) * chart_w).max(4.0);
        let itl_bar = ((m.itl_p50_ms / max_ttft) * chart_w).max(4.0);

        rows.push_str(&format!(
            r##"
    <g class="model-row">
      <text x="30" y="{y}" class="label-model">{name}</text>
      <text x="30" y="{desc_y}" class="label-topology">{topology} [{quant}]</text>
      <rect x="320" y="{rect_ttft_y}" width="{ttft_w}" height="14" rx="3" fill="#7fb8a6" />
      <rect x="320" y="{rect_itl_y}" width="{itl_w}" height="14" rx="3" fill="#6ba4d9" />
      <text x="{ttft_val_x}" y="{y_ttft_text}" class="label-stat">TTFT {ttft:.1}ms</text>
      <text x="{itl_val_x}" y="{y_itl_text}" class="label-stat">ITL {itl:.1}ms</text>
      <text x="820" y="{y}" class="badge-footprint">KV: {kv:.2} GB</text>
      <text x="960" y="{y}" class="badge-status">{status}</text>
    </g>"##,
            y = y,
            desc_y = y + 16.0,
            rect_ttft_y = y - 14.0,
            rect_itl_y = y + 2.0,
            ttft_w = ttft_bar,
            itl_w = itl_bar,
            ttft_val_x = 330.0 + ttft_bar,
            itl_val_x = 330.0 + itl_bar,
            y_ttft_text = y - 3.0,
            y_itl_text = y + 13.0,
            name = m.model_name,
            topology = m.architectural_topology,
            quant = m.quantization,
            ttft = m.ttft_p50_ms,
            itl = m.itl_p50_ms,
            kv = m.kv_footprint_gb,
            status = m.status,
        ));
    }

    let svg_h = (start_y + (data.multi_model_breadth.len() as f64) * row_h + 30.0) as u32;

    format!(
        r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 1100 {svg_h}" width="100%" height="100%">
  <defs>
    <style>
      .bg {{ fill: #18111f; }}
      .title {{ font-family: ui-sans-serif, system-ui, sans-serif; font-size: 20px; font-weight: 700; fill: #fff8ee; }}
      .subtitle {{ font-family: ui-sans-serif, system-ui, sans-serif; font-size: 13px; fill: #bca9c2; }}
      .label-model {{ font-family: ui-monospace, monospace; font-size: 13px; font-weight: 700; fill: #ded3df; }}
      .label-topology {{ font-family: ui-sans-serif, system-ui, sans-serif; font-size: 11px; fill: #bca9c2; }}
      .label-stat {{ font-family: ui-monospace, monospace; font-size: 11px; font-weight: 600; fill: #fff8ee; }}
      .badge-footprint {{ font-family: ui-monospace, monospace; font-size: 13px; font-weight: 700; fill: #d3a85b; }}
      .badge-status {{ font-family: ui-sans-serif, system-ui, sans-serif; font-size: 12px; font-weight: 800; fill: #7fb8a6; }}
    </style>
  </defs>
  <rect width="100%" height="100%" rx="12" class="bg" />
  <text x="30" y="48" class="title">Multi-Model Architecture Breadth & Silicon Scaling</text>
  <text x="30" y="74" class="subtitle">Empirical performance comparison across model topologies with pure compiled serving</text>
{rows}
</svg>"#,
        svg_h = svg_h,
        rows = rows
    )
}
