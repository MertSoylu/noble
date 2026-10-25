//! Command blocks in the app: moving through a pane's commands, copying the output of the last one and sending it to
//! an agent pane. The blocks come from the shell's OSC 133 marks (`term::osc`, `term::blocks`).

use super::*;
use crate::term::blocks::{agent_payload, context_line, trim_blank_tail};

impl App {
    /// The focused pane's view moves to the previous (`forward` false) or the next command.
    pub fn step_command(&mut self, forward: bool) {
        let Some(id) = self.focused_pane() else { return };
        let Some(pane) = self.panes.get(&id) else { return };
        let moved = pane.step_command(forward).is_some();
        let has_output = pane.last_output_block().is_some();
        if moved {
            return;
        }
        if !has_output {
            self.no_command_output();
        } else if forward {
            self.toast(ToastLevel::Info, "no later command in this pane");
        } else {
            self.toast(ToastLevel::Info, "no earlier command in this pane");
        }
    }

    /// Copies the output of the focused pane's last finished command.
    pub fn copy_last_output(&mut self) {
        let Some(id) = self.focused_pane() else { return };
        let Some(pane) = self.panes.get(&id) else { return };
        let lines = pane.last_output_block().and_then(|b| pane.block_lines(&b));
        let Some(mut lines) = lines else { return self.no_command_output() };
        trim_blank_tail(&mut lines);
        if lines.is_empty() {
            return self.toast(ToastLevel::Info, "the last command printed nothing");
        }
        self.set_clipboard(&lines.join("\n"), true);
    }

    /// Sends the focused pane's last finished command output to an agent pane: its selection instead when there is
    /// one, led by the context line of that command (`~/app $ cargo test → exit 101`). The text is pasted, never
    /// run: the user presses Enter. The target is the agent in the same tab, else the one last sent to.
    pub fn send_last_output(&mut self) {
        let Some(id) = self.focused_pane() else { return };
        let Some(pane) = self.panes.get(&id) else { return };
        let block = pane.last_output_block();
        let selected = pane.selected_text();
        let body = match (&selected, &block) {
            (Some(text), _) => text.lines().map(str::to_string).collect(),
            (None, Some(b)) => pane.block_lines(b).unwrap_or_default(),
            (None, None) => return self.no_command_output(),
        };
        let context = block.as_ref().map(|b| context_line(b.line.as_deref(), b.exit)).unwrap_or_default();
        let payload = agent_payload(&context, body);
        let Some(target) = self.agent_target(id) else {
            return self.toast(ToastLevel::Info, "no agent pane to send to: open one in this tab");
        };
        self.last_agent = Some(target);
        self.focus_pane(target);
        self.paste_into(target, payload);
    }

    /// The ✗ badge's click: the focused pane is the one clicked, and its view moves to the newest finished command.
    pub fn show_last_command(&mut self, pane: PaneId) {
        self.focus_pane(pane);
        if !self.panes.get(&pane).is_some_and(|p| p.show_last_output()) {
            self.no_command_output();
        }
    }

    /// The agent pane that receives a command's output: the first agent in the pane's tab, else the one last sent to.
    pub(super) fn agent_target(&self, from: PaneId) -> Option<PaneId> {
        let agent = |p: &PaneId| *p != from && self.panes.contains_key(p) && self.agent_state(*p).is_some();
        let in_tab =
            self.tabs.iter().find(|t| t.root.contains(from)).and_then(|t| t.panes().into_iter().find(|p| agent(p)));
        in_tab.or_else(|| self.last_agent.filter(|p| agent(p)))
    }

    fn no_command_output(&mut self) {
        self.toast(
            ToastLevel::Info,
            "no command output in this pane (the shell sends no command marks, or nothing ran yet)",
        );
    }
}
