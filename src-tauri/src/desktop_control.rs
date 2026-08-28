use crate::app_state::{Agent, ApplicationState};

#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct PressedInputTracker {
    pressed: Vec<i32>,
}

impl PressedInputTracker {
    pub fn record_pressed(&mut self, code: i32) {
        if !self.pressed.contains(&code) {
            self.pressed.push(code);
        }
    }

    pub fn record_released(&mut self, code: i32) {
        if let Some(index) = self.pressed.iter().rposition(|pressed| *pressed == code) {
            self.pressed.remove(index);
        }
    }

    pub fn release_order(&self) -> impl Iterator<Item = i32> + '_ {
        self.pressed.iter().rev().copied()
    }

    pub fn is_empty(&self) -> bool {
        self.pressed.is_empty()
    }
}

pub fn agent_retains_desktop_control(agent: &Agent) -> bool {
    agent.registry_state == "active"
        && agent.template_key.as_deref() == Some("pc-control")
        && agent.capabilities.system == "full"
}

pub fn state_retains_desktop_control(state: &ApplicationState, agent_id: i64) -> bool {
    state
        .agents
        .iter()
        .find(|agent| agent.id == agent_id)
        .is_some_and(agent_retains_desktop_control)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn task_0016_pressed_inputs_release_in_reverse_without_duplicates() {
        let mut tracker = PressedInputTracker::default();
        tracker.record_pressed(1);
        tracker.record_pressed(2);
        tracker.record_pressed(2);
        assert_eq!(tracker.release_order().collect::<Vec<_>>(), [2, 1]);
        tracker.record_released(2);
        assert_eq!(tracker.release_order().collect::<Vec<_>>(), [1]);
        tracker.record_released(1);
        assert!(tracker.is_empty());
    }

    #[test]
    fn task_0016_desktop_control_requires_the_exact_active_full_pc_agent() {
        let mut state = crate::app_state::default_application_state().unwrap();
        let agent = state
            .agents
            .iter_mut()
            .find(|agent| agent.template_key.as_deref() == Some("pc-control"))
            .unwrap();
        let agent_id = agent.id;
        agent.capabilities.system = "full".to_string();
        assert!(state_retains_desktop_control(&state, agent_id));

        state
            .agents
            .iter_mut()
            .find(|agent| agent.id == agent_id)
            .unwrap()
            .capabilities
            .system = "power".to_string();
        assert!(!state_retains_desktop_control(&state, agent_id));
        assert!(!state_retains_desktop_control(&state, i64::MAX));
    }
}
