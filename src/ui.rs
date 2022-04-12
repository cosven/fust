use crate::app::App;
use crate::player::PlayerState;
use std::time::Duration;
use tui::{
    backend::Backend,
    layout::{Alignment, Constraint, Direction, Layout},
    style::{Color, Modifier, Style},
    symbols::line::THICK,
    symbols::DOT,
    text::{Span, Spans},
    widgets::{LineGauge, Paragraph, Wrap},
    Frame,
};

fn fmt_duration(duration: Duration) -> String {
    let seconds = duration.as_secs() % 60;
    let minutes = (duration.as_secs() / 60) % 60;
    let hours = (duration.as_secs() / 60) / 60;
    if hours > 0 {
        format!("{:0>2}:{:0>2}:{:0>2}", hours, minutes, seconds)
    } else {
        format!("{:0>2}:{:0>2}", minutes, seconds)
    }
}

pub fn ui<B: Backend>(f: &mut Frame<B>, app: &App) {
    let area = f.size();
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .margin(1)
        .constraints(
            [
                Constraint::Min(1),
                Constraint::Length(1),
                Constraint::Length(1),
                Constraint::Length(1),
            ]
            .as_ref(),
        )
        .split(area);

    let inner = app.inner.lock().unwrap();
    let metadata = inner.metadata.clone();
    let lyric_s = inner.lyric_s.clone();
    let position = inner.progress.current();
    let duration = inner.duration;
    let state = inner.state;
    drop(inner);

    let mut song_spans = vec![
        Span::raw(" ".to_owned()),
        Span::styled("♫  ", Style::default().fg(Color::Yellow)),
        Span::raw(metadata.title),
    ];
    if !metadata.artists.is_empty() {
        song_spans.push(Span::raw(DOT));
        song_spans.push(Span::styled(DOT, Style::default().fg(Color::Gray)));
        song_spans.push(Span::raw(metadata.artists.join(",")));
    }

    let color = match state {
        PlayerState::Stopped => Color::Gray,
        PlayerState::Paused => Color::Gray,
        PlayerState::Playing => Color::LightCyan,
    };
    let ratio = match duration.as_secs_f64() <= 0.0 {
        true => 0.0,
        false => {
            let ratio = position.as_secs_f64() / duration.as_secs_f64();
            if ratio >= 1.0 {
                1.0
            } else {
                ratio
            }
        }
    };
    let progress = LineGauge::default()
        .gauge_style(Style::default().fg(color))
        .label(Span::styled(
            format!("[{}/{}]", fmt_duration(position), fmt_duration(duration)),
            Style::default().fg(color).add_modifier(Modifier::ITALIC),
        ))
        .line_set(THICK)
        .ratio(ratio);
    f.render_widget(progress, chunks[2]);

    let lyric = Paragraph::new(vec![Spans::from(lyric_s)])
        .wrap(Wrap { trim: true })
        .alignment(Alignment::Right);
    let song = Paragraph::new(Spans::from(song_spans)).wrap(Wrap { trim: true });
    let h_chunks = Layout::default()
        .direction(Direction::Horizontal)
        .margin(0)
        .constraints([Constraint::Percentage(50), Constraint::Percentage(50)].as_ref())
        .split(chunks[3]);
    f.render_widget(song, h_chunks[0]);
    f.render_widget(lyric, h_chunks[1]);
}
