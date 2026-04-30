use anyhow::Result;
use chrono::Utc;

use crate::store::State;
use crate::time_input::parse_duration;

/// Set DND for a given duration.
pub fn set_dnd(duration_str: &str) -> Result<()> {
    let duration = parse_duration(duration_str)?;
    let until = Utc::now() + duration;

    let mut state = State::load()?;
    state.set_dnd(until);
    state.save()?;

    println!("DND set until {}", until.format("%Y-%m-%d %H:%M:%S UTC"));
    Ok(())
}

/// Clear DND.
pub fn clear_dnd() -> Result<()> {
    let mut state = State::load()?;
    state.clear_dnd();
    state.save()?;

    println!("DND cleared");
    Ok(())
}

/// Show DND status.
pub fn show_dnd_status() -> Result<()> {
    let state = State::load()?;

    if state.is_dnd_active() {
        if let Some(until) = state.dnd_until {
            let remaining = until - Utc::now();
            let hours = remaining.num_hours();
            let mins = remaining.num_minutes() % 60;
            println!(
                "DND active until {} ({:02}h {:02}m remaining)",
                until.format("%Y-%m-%d %H:%M:%S UTC"),
                hours,
                mins
            );
        }
    } else {
        println!("DND is not active");
    }

    Ok(())
}
