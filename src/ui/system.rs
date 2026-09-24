//! SYSTEM: CPU history and cores, memory/network/disks, sortable process table.

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};

use super::hud;
use crate::app::{App, Hit, SortKey};
use crate::util;

pub fn draw(buf: &mut Buffer, area: Rect, app: &App, hits: &mut Vec<(Rect, Hit)>) {
    let th = &app.theme;
    if area.width < 30 || area.height < 8 {
        hud::put_center(buf, area, area.y + area.height / 2, "enlarge window", th.dim());
        return;
    }
    let area = if area.width >= 80 { Rect::new(area.x + 1, area.y, area.width - 2, area.height) } else { area };
    let top_h = if area.height >= 30 { area.height * 2 / 5 } else { (area.height / 2).max(8) }.min(area.height - 5);
    let top = Rect::new(area.x, area.y, area.width, top_h);
    let procs_area = Rect::new(area.x, area.y + top_h, area.width, area.height - top_h);
    if area.width >= 90 {
        let cpu_w = area.width * 3 / 5;
        cpu(buf, Rect::new(top.x, top.y, cpu_w, top.height), app);
        memory(buf, Rect::new(top.x + cpu_w + 1, top.y, top.width - cpu_w - 1, top.height), app);
    } else {
        cpu(buf, top, app);
    }
    processes(buf, procs_area, app, hits);
}

fn cpu(buf: &mut Buffer, area: Rect, app: &App) {
    let th = &app.theme;
    let s = &app.sensors;
    let tag = match &s.last {
        Some(last) => {
            let ghz =
                if last.freq_mhz > 0 { format!(" · {:.1} GHz", last.freq_mhz as f64 / 1000.0) } else { String::new() };
            format!("{:.0}% · {} cores{ghz}", last.cpu, s.info.cores)
        }
        None => "warming up".into(),
    };
    let inner = hud::frame(buf, area, "CPU", &tag, false, th);
    if inner.height < 2 || inner.width < 10 {
        return;
    }
    let x = inner.x + 1;
    let w = inner.width.saturating_sub(2);
    hud::put(buf, x, inner.y, &util::truncate(&s.info.cpu_brand, w as usize), th.dim(), w);
    let Some(last) = &s.last else { return };
    let cores = last.cores.len().max(1);
    let col_w: u16 = 16;
    let cols = (w / col_w).clamp(1, 8) as usize;
    let core_rows = cores.div_ceil(cols) as u16;
    let avail = inner.height.saturating_sub(1);
    let core_rows = if avail >= core_rows + 3 { core_rows } else { 0 };
    let graph_h = avail.saturating_sub(core_rows + if core_rows > 0 { 1 } else { 0 });
    if graph_h > 0 {
        let vals: Vec<f64> = s.cpu_hist.iter().map(|v| *v as f64 / 100.0).collect();
        hud::braille_area(buf, Rect::new(x, inner.y + 1, w, graph_h), &vals, th.accent_dim, th.accent);
    }
    if core_rows > 0 {
        let y0 = inner.y + 1 + graph_h + 1;
        let cell_w = w / cols as u16;
        for (i, v) in last.cores.iter().enumerate() {
            let (row, col) = ((i / cols) as u16, (i % cols) as u16);
            let cx = x + col * cell_w;
            let y = y0 + row;
            let pct = *v as f64;
            hud::put(buf, cx, y, &format!("{:>2}", i + 1), th.dim(), 2);
            let bar_w = cell_w.saturating_sub(2 + 1 + 5 + 1);
            hud::bar(buf, cx + 3, y, bar_w, pct, th.level(pct), th);
            hud::put(buf, cx + 3 + bar_w, y, &format!("{pct:>4.0}%"), Style::default().fg(th.level(pct)), 5);
        }
    }
}

fn memory(buf: &mut Buffer, area: Rect, app: &App) {
    let th = &app.theme;
    let s = &app.sensors;
    let inner = hud::frame(buf, area, "MEMORY · NET · DISK", "", false, th);
    if inner.height < 2 || inner.width < 16 {
        return;
    }
    let Some(last) = &s.last else { return };
    let x = inner.x + 1;
    let w = inner.width.saturating_sub(2);
    // Battery at the very bottom; memory, network and disks stay above it.
    let bat_h = super::battery_rows(app, inner.height + 4);
    if bat_h > 0 {
        let by = inner.bottom() - bat_h;
        super::battery_block(buf, Rect::new(x, by, w, bat_h), app);
    }
    let bottom = inner.bottom() - if bat_h > 0 { bat_h + 1 } else { 0 };
    let mut y = inner.y;
    let gauge = |buf: &mut Buffer, y: u16, label: &str, used: u64, total: u64| {
        let pct = if total > 0 { used as f64 / total as f64 * 100.0 } else { 0.0 };
        hud::put(buf, x, y, label, th.dim(), 5);
        let right = format!("{} / {}", util::fmt_bytes(used), util::fmt_bytes(total));
        let bar_w = w.saturating_sub(5 + util::width(&right) as u16 + 1);
        hud::bar(buf, x + 5, y, bar_w, pct, th.level(pct), th);
        hud::put_right(buf, x + w, y, &right, th.text());
    };
    gauge(buf, y, "RAM", last.mem_used, last.mem_total);
    y += 1;
    if last.swap_total > 0 && y < bottom {
        gauge(buf, y, "SWAP", last.swap_used, last.swap_total);
        y += 1;
    }
    y += 1;
    let max = s.rx_hist.iter().chain(s.tx_hist.iter()).fold(64.0 * 1024.0, |m, v| if *v > m { *v } else { m });
    let disk_rows = last.disks.len().min(4) as u16;
    let net_rows = bottom.saturating_sub(y + 2 + disk_rows + 1);
    if net_rows >= 2 && y + 1 < bottom {
        hud::put_spans(
            buf,
            x,
            y,
            &[
                ("↓ ", Style::default().fg(th.accent2)),
                (&util::fmt_rate(last.rx_rate), th.text()),
                ("   ↑ ", Style::default().fg(th.accent)),
                (&util::fmt_rate(last.tx_rate), th.text()),
            ],
            w,
        );
        hud::put_right(buf, x + w, y, &format!("peak {}", util::fmt_rate(max)), th.dim());
        y += 1;
        let half = net_rows / 2;
        let rx: Vec<f64> = s.rx_hist.iter().map(|v| (v / max).sqrt()).collect();
        let tx: Vec<f64> = s.tx_hist.iter().map(|v| (v / max).sqrt()).collect();
        hud::braille_area(buf, Rect::new(x, y, w, half.max(1)), &rx, th.line, th.accent2);
        y += half.max(1);
        if net_rows - half > 0 {
            hud::braille_area(buf, Rect::new(x, y, w, (net_rows - half).max(1)), &tx, th.line, th.accent);
            y += net_rows - half;
        }
        y += 1;
    }
    for d in last.disks.iter().take(4) {
        if y >= bottom {
            break;
        }
        let label = util::truncate(&d.mount, 4);
        let pct = if d.total > 0 { d.used as f64 / d.total as f64 * 100.0 } else { 0.0 };
        hud::put(buf, x, y, &label, th.dim(), 5);
        let right = format!("{} free", util::fmt_bytes(d.total.saturating_sub(d.used)));
        let bar_w = w.saturating_sub(5 + util::width(&right) as u16 + 1);
        hud::bar(buf, x + 5, y, bar_w, pct, th.level(pct), th);
        hud::put_right(buf, x + w, y, &right, th.dim());
        y += 1;
    }
}

fn processes(buf: &mut Buffer, area: Rect, app: &App, hits: &mut Vec<(Rect, Hit)>) {
    let th = &app.theme;
    let procs = app.visible_procs();
    let total = app.sensors.last.as_ref().map(|s| s.proc_count).unwrap_or(0);
    let arrow = if app.system.desc { "▼" } else { "▲" };
    let sort_name = match app.system.sort {
        SortKey::Cpu => "CPU",
        SortKey::Mem => "MEM",
        SortKey::Pid => "PID",
        SortKey::Name => "NAME",
    };
    let tag = if app.system.filter.is_empty() {
        format!("{total} · sort {sort_name} {arrow}")
    } else {
        format!("{}/{total} · sort {sort_name} {arrow}", procs.len())
    };
    let inner = hud::frame(buf, area, "PROCESSES", &tag, true, th);
    if inner.height < 3 || inner.width < 24 {
        return;
    }
    let x = inner.x + 1;
    let w = inner.width.saturating_sub(2);
    let mut y = inner.y;
    if app.system.filtering || !app.system.filter.is_empty() {
        let cursor = if app.system.filtering { "▏" } else { "" };
        hud::put_spans(
            buf,
            x,
            y,
            &[("/ ", th.accent()), (&app.system.filter, th.accent_bold()), (cursor, th.accent())],
            w,
        );
        y += 1;
    }
    // Columns: PID | NAME | CPU% + bar | MEM
    let pid_w = 8u16;
    let cpu_w = 7u16;
    let bar_w = if w >= 70 { 12u16 } else { 0 };
    let mem_w = 9u16;
    let name_w = w.saturating_sub(pid_w + cpu_w + 1 + bar_w + mem_w);
    let header = |key: SortKey, label: &str| {
        let active = app.system.sort == key;
        let text = if active { format!("{label}{arrow}") } else { label.to_string() };
        let style = if active { th.accent_bold() } else { th.dim().add_modifier(Modifier::BOLD) };
        (text, style)
    };
    let cols = [
        (SortKey::Pid, "PID", pid_w, true),
        (SortKey::Name, "NAME", name_w, false),
        (SortKey::Cpu, "CPU%", cpu_w, true),
    ];
    let mut cx = x;
    for (key, label, cw, right) in cols {
        let (text, style) = header(key, label);
        let text =
            if right { util::pad_left(&text, cw as usize - 1) + " " } else { util::pad_right(&text, cw as usize) };
        hud::put(buf, cx, y, &text, style, cw);
        hits.push((Rect::new(cx, y, cw, 1), Hit::SortCol(key)));
        cx += cw;
    }
    cx += 1 + bar_w;
    let (text, style) = header(SortKey::Mem, "MEM");
    hud::put(buf, cx, y, &util::pad_left(&text, mem_w as usize), style, mem_w);
    hits.push((Rect::new(cx, y, mem_w, 1), Hit::SortCol(SortKey::Mem)));
    y += 1;

    let rows = inner.bottom().saturating_sub(y + 1) as usize;
    if procs.is_empty() {
        let msg = if app.sensors.last.is_none() { "sampling processes…" } else { "no process matches the filter" };
        hud::put(buf, x, y, msg, th.dim(), w);
    }
    let sel = app.system.selected_pid.and_then(|pid| procs.iter().position(|p| p.pid == pid));
    let offset = match sel {
        Some(s) if s >= rows => s + 1 - rows,
        _ => 0,
    };
    for (i, p) in procs.iter().enumerate().skip(offset).take(rows) {
        let ry = y + (i - offset) as u16;
        let selected = Some(i) == sel;
        let bgc = if selected { th.sel_bg } else { th.bg };
        if selected {
            hud::set_bg_row(buf, inner.x, ry, inner.width, th.sel_bg);
        }
        let cpu = p.cpu as f64;
        let mut cx = x;
        cx = hud::put(buf, cx, ry, &util::pad_left(&p.pid.to_string(), pid_w as usize - 1), th.dim().bg(bgc), pid_w);
        cx = hud::put(buf, cx, ry, " ", th.dim().bg(bgc), 1);
        let name_style = if selected { th.accent_bold().bg(bgc) } else { th.text().bg(bgc) };
        cx = hud::put(buf, cx, ry, &util::pad_right(&p.name, name_w as usize), name_style, name_w);
        cx = hud::put(
            buf,
            cx,
            ry,
            &util::pad_left(&format!("{cpu:.1}"), cpu_w as usize),
            Style::default().fg(th.level(cpu * 2.0)).bg(bgc),
            cpu_w,
        );
        cx += 1;
        if bar_w > 0 {
            hud::bar(buf, cx, ry, bar_w, cpu.min(100.0) * 2.0, th.level(cpu * 2.0), th);
            cx += bar_w;
        }
        hud::put(buf, cx, ry, &util::pad_left(&util::fmt_bytes(p.mem), mem_w as usize), th.text().bg(bgc), mem_w);
        hits.push((Rect::new(inner.x, ry, inner.width, 1), Hit::Proc(p.pid)));
    }
    let footer = "↑↓ select · c m p n sort · / filter · K terminate · esc back";
    hud::put(buf, x, inner.bottom() - 1, &util::truncate(footer, w as usize), th.line(), w);
}
