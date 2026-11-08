//! Authoritative first-capture coaching. Renderer actions cannot fake capture events.
use serde::Serialize;
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all="kebab-case")]
pub enum Stage { Start, Screen, Whiteboard, BoardInk, ScreenInk, Stop, Preparing, Copied, Paste, Locate, Complete, Retry, CopyFailed }
#[derive(Debug)]
pub struct Practice { pub stage: Stage, board_ink: bool, screen_ink: bool, pub latest: bool, screen_baseline: usize, pasted: bool }
impl Default for Practice { fn default()->Self { Self{stage:Stage::Start,board_ink:false,screen_ink:false,latest:false,screen_baseline:0,pasted:false} } }
impl Practice {
    pub fn selecting(&mut self) { if self.stage!=Stage::Screen {*self=Self::default();self.stage=Stage::Screen;} }
    pub fn started(&mut self) { if matches!(self.stage,Stage::Start|Stage::Screen|Stage::Retry|Stage::CopyFailed) {*self=Self::default();self.stage=Stage::Whiteboard;} }
    pub fn whiteboard(&mut self) { if self.stage==Stage::Whiteboard {self.stage=Stage::BoardInk;} }
    pub fn whiteboard_visible(&mut self, visible: bool) {
        if visible { self.whiteboard(); }
        else if self.stage == Stage::BoardInk { self.stage=Stage::Whiteboard; }
    }
    pub fn observe_ink(&mut self,board:bool,screen:usize) {
        if self.stage==Stage::BoardInk && board {self.board_ink=true;self.screen_baseline=screen;self.stage=Stage::ScreenInk;return;}
        if self.stage==Stage::ScreenInk && screen>self.screen_baseline {self.screen_ink=true;self.stage=Stage::Stop;}
    }
    pub fn processing(&mut self) { if matches!(self.stage,Stage::Whiteboard|Stage::BoardInk|Stage::ScreenInk|Stage::Stop) {self.stage=Stage::Preparing;} }
    pub fn saved(&mut self,copied:bool) {
        if self.stage!=Stage::Preparing {return;}
        self.stage=if copied {Stage::Copied} else {Stage::CopyFailed};
    }
    pub fn cancelled(&mut self) { if matches!(self.stage,Stage::Screen|Stage::Whiteboard|Stage::BoardInk|Stage::ScreenInk|Stage::Stop|Stage::Preparing) {self.stage=Stage::Retry;} }
    // A person confirms their manual paste. A key press is never treated as proof
    // of delivery to another application; no monitoring of a private composer.
    pub fn action(&mut self,action:&str)->bool {
        match (self.stage,action) {
            (Stage::Copied,"practice-paste")=>{self.stage=Stage::Paste;true},
            (Stage::Paste,"practice-pasted")=>{self.pasted=true;self.stage=Stage::Locate;true},
            (Stage::CopyFailed,"practice-locate")=>{self.stage=Stage::Locate;true},
            (Stage::Complete,"practice-finish")=>true,
            _=>false,
        }
    }
    pub fn opened_latest(&mut self) {if self.stage==Stage::Locate {self.latest=true;self.stage=if self.board_ink&&self.screen_ink&&self.pasted {Stage::Complete} else {Stage::Retry};}}
}
#[cfg(test)] mod tests {
    use super::*;
    #[test] fn renderer_cannot_skip_real_capture_or_drawing() {
        let mut p=Practice::default();assert!(!p.action("practice-pasted"));p.started();p.observe_ink(true,1);assert_eq!(p.stage,Stage::Whiteboard);
        p.whiteboard();p.observe_ink(true,0);assert_eq!(p.stage,Stage::ScreenInk);p.observe_ink(true,1);assert_eq!(p.stage,Stage::Stop);
        p.processing();p.saved(true);assert_eq!(p.stage,Stage::Copied);assert!(!p.action("practice-finish"));assert!(p.action("practice-paste"));assert!(p.action("practice-pasted"));p.opened_latest();assert_eq!(p.stage,Stage::Complete);assert!(p.action("practice-finish"));
    }
    #[test] fn owned_board_visibility_reconciles_open_and_close_without_skipping_ink() {
        let mut p=Practice::default();
        p.whiteboard_visible(true);assert_eq!(p.stage,Stage::Start);
        p.started();p.whiteboard_visible(false);assert_eq!(p.stage,Stage::Whiteboard);
        p.whiteboard_visible(true);assert_eq!(p.stage,Stage::BoardInk);
        p.whiteboard_visible(false);assert_eq!(p.stage,Stage::Whiteboard);
        p.whiteboard_visible(true);p.observe_ink(true,0);assert_eq!(p.stage,Stage::ScreenInk);
        p.whiteboard_visible(false);assert_eq!(p.stage,Stage::ScreenInk);
    }
    #[test] fn failed_copy_and_early_stop_never_claim_copied() {
        let mut p=Practice::default();p.started();p.processing();p.saved(true);assert_eq!(p.stage,Stage::Copied);
        p.selecting();p.started();p.whiteboard();p.observe_ink(true,0);p.observe_ink(true,1);p.processing();p.saved(false);assert_eq!(p.stage,Stage::CopyFailed);
        p=Practice::default();p.started();p.cancelled();assert_eq!(p.stage,Stage::Retry);
    }
}

#[cfg(test)] mod saved_lesson_tests {
    use super::*;
    #[test] fn partial_saved_capture_still_teaches_where_the_app_lives_without_completing_practice() {
        let mut p=Practice::default();p.started();p.whiteboard();p.observe_ink(true,0);
        p.processing();p.saved(true);
        assert_eq!(p.stage,Stage::Copied,"a real copied capture must still lead to the paste/location lesson");
        assert!(p.action("practice-paste"));assert!(p.action("practice-pasted"));
        assert_eq!(p.stage,Stage::Locate);p.opened_latest();
        assert_eq!(p.stage,Stage::Retry,"missing screen ink must not complete the practice");
        assert!(!p.action("practice-finish"));
    }
}

#[cfg(test)] mod picker_tests {
    use super::*;
    #[test] fn screen_is_taught_before_whiteboard_and_cancellation_cannot_complete_practice() {
        let mut p=Practice::default();p.selecting();assert_eq!(p.stage,Stage::Screen);
        p.processing();p.saved(true);assert_eq!(p.stage,Stage::Screen,"a saved receipt cannot skip real recording start");
        p.whiteboard_visible(true);assert_eq!(p.stage,Stage::Screen);
        p.started();assert_eq!(p.stage,Stage::Whiteboard);
        p.cancelled();assert_eq!(p.stage,Stage::Retry);assert!(!p.action("practice-finish"));
    }
    #[test] fn failed_copy_can_find_the_saved_capture_but_cannot_claim_a_paste() {
        let mut p=Practice::default();p.started();p.whiteboard();p.observe_ink(true,0);p.observe_ink(true,1);
        p.processing();p.saved(false);assert_eq!(p.stage,Stage::CopyFailed);
        assert!(p.action("practice-locate"));assert_eq!(p.stage,Stage::Locate);p.opened_latest();
        assert_eq!(p.stage,Stage::Retry);assert!(!p.action("practice-finish"));
    }
}
