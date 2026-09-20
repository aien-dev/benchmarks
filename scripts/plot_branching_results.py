#!/usr/bin/env python3
"""
Plotting engine for Real-Model Agentic Branching Benchmark results on DGX Spark GB10.
Consumes summary.json and renders standalone publication SVG and PNG comparison charts.
"""

import argparse
import json
import os
import sys
from pathlib import Path
from typing import Dict, List, Any

def generate_svg_chart(
    title: str,
    x_label: str,
    y_label: str,
    data_series: Dict[str, List[tuple]],
    colors: Dict[str, str],
    output_path: Path,
    y_is_log: bool = False,
):
    width = 900
    height = 540
    padding_left = 90
    padding_right = 160
    padding_top = 70
    padding_bottom = 70

    plot_w = width - padding_left - padding_right
    plot_h = height - padding_top - padding_bottom

    all_x = []
    all_y = []
    for s_name, pts in data_series.items():
        for x, y in pts:
            all_x.append(x)
            all_y.append(y)

    if not all_x or not all_y:
        return

    min_x = min(all_x)
    max_x = max(all_x)
    min_y = 0.0 if not y_is_log else max(1e-3, min(all_y))
    max_y = max(all_y) * 1.15

    def scale_x(val):
        if max_x == min_x:
            return padding_left + plot_w / 2
        return padding_left + ((val - min_x) / (max_x - min_x)) * plot_w

    def scale_y(val):
        if max_y == min_y:
            return padding_top + plot_h / 2
        return padding_top + plot_h - ((val - min_y) / (max_y - min_y)) * plot_h

    svg = []
    svg.append(f'<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 {width} {height}" width="{width}" height="{height}">')
    svg.append('  <style>')
    svg.append('    text { font-family: -apple-system, BlinkMacSystemFont, "Segoe UI", Roboto, sans-serif; }')
    svg.append('    .axis-line { stroke: #444; stroke-width: 1.5; }')
    svg.append('    .grid-line { stroke: #262626; stroke-width: 1; stroke-dasharray: 4,4; }')
    svg.append('    .title { font-size: 18px; font-weight: 600; fill: #ededed; }')
    svg.append('    .axis-label { font-size: 13px; fill: #a1a1aa; }')
    svg.append('    .tick-label { font-size: 11px; fill: #71717a; }')
    svg.append('    .legend-text { font-size: 12px; fill: #d4d4d8; }')
    svg.append('  </style>')
    svg.append('  <rect width="100%" height="100%" fill="#0d1117" />')

    # Title
    svg.append(f'  <text x="{padding_left}" y="36" class="title">{title}</text>')

    # Grid & Y ticks
    num_y_ticks = 5
    for i in range(num_y_ticks + 1):
        y_val = min_y + (i / num_y_ticks) * (max_y - min_y)
        y_pos = scale_y(y_val)
        svg.append(f'  <line x1="{padding_left}" y1="{y_pos}" x2="{padding_left + plot_w}" y2="{y_pos}" class="grid-line" />')
        svg.append(f'  <text x="{padding_left - 12}" y="{y_pos + 4}" text-anchor="end" class="tick-label">{y_val:.1f}</text>')

    # X ticks
    x_ticks = sorted(list(set(all_x)))
    for x_val in x_ticks:
        x_pos = scale_x(x_val)
        svg.append(f'  <line x1="{x_pos}" y1="{padding_top}" x2="{x_pos}" y2="{padding_top + plot_h}" class="grid-line" />')
        svg.append(f'  <text x="{x_pos}" y="{padding_top + plot_h + 20}" text-anchor="middle" class="tick-label">{int(x_val)}</text>')

    # Axes
    svg.append(f'  <line x1="{padding_left}" y1="{padding_top + plot_h}" x2="{padding_left + plot_w}" y2="{padding_top + plot_h}" class="axis-line" />')
    svg.append(f'  <line x1="{padding_left}" y1="{padding_top}" x2="{padding_left}" y2="{padding_top + plot_h}" class="axis-line" />')

    # Axis labels
    svg.append(f'  <text x="{padding_left + plot_w / 2}" y="{height - 20}" text-anchor="middle" class="axis-label">{x_label}</text>')
    svg.append(f'  <text transform="rotate(-90)" x="{-padding_top - plot_h / 2}" y="28" text-anchor="middle" class="axis-label">{y_label}</text>')

    # Series plots
    legend_y = padding_top + 10
    for s_name, pts in data_series.items():
        color = colors.get(s_name, "#38bdf8")
        pts_sorted = sorted(pts, key=lambda p: p[0])
        polyline_pts = []
        for x, y in pts_sorted:
            px = scale_x(x)
            py = scale_y(y)
            polyline_pts.append(f'{px:.1f},{py:.1f}')
            svg.append(f'  <circle cx="{px:.1f}" cy="{py:.1f}" r="4.5" fill="{color}" />')

        if len(polyline_pts) > 1:
            svg.append(f'  <polyline points="{" ".join(polyline_pts)}" fill="none" stroke="{color}" stroke-width="2.5" />')

        # Legend entry
        legend_x = padding_left + plot_w + 24
        svg.append(f'  <circle cx="{legend_x}" cy="{legend_y}" r="5" fill="{color}" />')
        svg.append(f'  <text x="{legend_x + 14}" y="{legend_y + 4}" class="legend-text">{s_name}</text>')
        legend_y += 24

    svg.append('</svg>')

    with open(output_path, "w", encoding="utf-8") as f:
        f.write("\n".join(svg))
    print(f"Rendered: {output_path}")

def main():
    parser = argparse.ArgumentParser(description="Render benchmark SVG charts from summary.json")
    parser.add_argument("--input", required=True, help="Path to summary.json")
    parser.add_argument("--output-dir", required=True, help="Directory to save rendered SVG files")
    args = parser.parse_args()

    input_path = Path(args.input)
    output_dir = Path(args.output_dir)
    output_dir.mkdir(parents=True, exist_ok=True)

    with open(input_path, "r") as f:
        data = json.load(f)

    colors = {
        "AIEN (COW Paged KV)": "#10b981",
        "Modular MAX": "#f59e0b",
        "vLLM": "#6366f1",
        "aien": "#10b981",
        "max": "#f59e0b",
        "vllm": "#6366f1",
    }

    # Group by engine
    by_engine: Dict[str, List[Dict[str, Any]]] = {}
    for entry in data:
        eng = entry.get("engine", "unknown")
        by_engine.setdefault(eng, []).append(entry)

    # 1. p95 TTFT vs Branches
    ttft_series = {}
    for eng, entries in by_engine.items():
        pts = [(e["branch_count"], e["ttft_p95_ms"]) for e in entries if "ttft_p95_ms" in e]
        if pts:
            ttft_series[eng] = pts

    generate_svg_chart(
        title="Time to First Token (p95 TTFT) vs Concurrent Branches (32K Prefix)",
        x_label="Concurrent Agent Branches",
        y_label="TTFT p95 (milliseconds)",
        data_series=ttft_series,
        colors=colors,
        output_path=output_dir / "ttft_p95_vs_branches.svg",
    )

    # 2. Aggregate Throughput vs Branches
    throughput_series = {}
    for eng, entries in by_engine.items():
        pts = [(e["branch_count"], e["aggregate_tokens_per_sec"]) for e in entries if "aggregate_tokens_per_sec" in e]
        if pts:
            throughput_series[eng] = pts

    generate_svg_chart(
        title="Aggregate Generation Throughput vs Concurrent Branches (32K Prefix)",
        x_label="Concurrent Agent Branches",
        y_label="Aggregate Tokens / Second",
        data_series=throughput_series,
        colors=colors,
        output_path=output_dir / "throughput_vs_branches.svg",
    )

    # 3. PSS Memory vs Branches
    pss_series = {}
    for eng, entries in by_engine.items():
        pts = [(e["branch_count"], (e["pss_mb_peak"] or 0.0) / 1024.0) for e in entries if e.get("pss_mb_peak")]
        if pts:
            pss_series[eng] = pts

    if pss_series:
        generate_svg_chart(
            title="Unified Memory Footprint (PSS in GiB) vs Concurrent Branches",
            x_label="Concurrent Agent Branches",
            y_label="Process Proportional Set Size (GiB)",
            data_series=pss_series,
            colors=colors,
            output_path=output_dir / "memory_pss_vs_branches.svg",
        )

if __name__ == "__main__":
    main()
