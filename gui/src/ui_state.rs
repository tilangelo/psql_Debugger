//! Фаза работы окна. Намерение закрыть окно хранится отдельно от фазы.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum UiState {
    #[default]
    Idle,
    LoadingCatalog,
    StartingDemo,
    Running,
    Paused,
    Stopping,
    ClosingDemo,
}
#[derive(Clone, Copy, Debug)]
pub enum Action {
    LoadCatalog,
    StartDemo,
    Loaded,
    StartDebug,
    Pause,
    Resume,
    Stop,
    Finished,
    CloseDemo,
    DemoClosed,
}
impl UiState {
    pub fn transition(self, action: Action) -> Option<Self> {
        use Action::*;
        match (self, action) {
            (Self::Idle, LoadCatalog) => Some(Self::LoadingCatalog),
            (Self::Idle, StartDemo) => Some(Self::StartingDemo),
            (Self::LoadingCatalog | Self::StartingDemo, Loaded) => Some(Self::Idle),
            (Self::Idle, StartDebug) => Some(Self::Running),
            (Self::Running, Pause) => Some(Self::Paused),
            (Self::Paused, Resume) => Some(Self::Running),
            (Self::Running | Self::Paused, Stop) => Some(Self::Stopping),
            (Self::Running | Self::Paused | Self::Stopping, Finished) => Some(Self::Idle),
            (Self::Idle, CloseDemo) => Some(Self::ClosingDemo),
            (Self::ClosingDemo, DemoClosed) => Some(Self::Idle),
            _ => None,
        }
    }
    pub fn is_busy(self) -> bool {
        matches!(
            self,
            Self::LoadingCatalog | Self::StartingDemo | Self::ClosingDemo
        )
    }
    pub fn is_debugging(self) -> bool {
        matches!(self, Self::Running | Self::Paused | Self::Stopping)
    }
    pub fn can_edit_points(self) -> bool {
        matches!(self, Self::Idle | Self::Paused)
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn normal_debugging_and_cancellation() {
        let mut state = UiState::Idle;
        for action in [
            Action::LoadCatalog,
            Action::Loaded,
            Action::StartDebug,
            Action::Pause,
            Action::Resume,
            Action::Pause,
            Action::Stop,
            Action::Finished,
            Action::CloseDemo,
            Action::DemoClosed,
        ] {
            state = state.transition(action).unwrap();
        }
        assert_eq!(state, UiState::Idle);
    }
    #[test]
    fn late_pause_cannot_revive_stopping_session() {
        assert_eq!(UiState::Stopping.transition(Action::Pause), None);
        assert_eq!(UiState::Stopping.transition(Action::StartDebug), None);
        assert!(!UiState::Stopping.can_edit_points());
        assert_eq!(UiState::ClosingDemo.transition(Action::StartDemo), None);
        assert_eq!(UiState::LoadingCatalog.transition(Action::StartDebug), None);
    }
}
