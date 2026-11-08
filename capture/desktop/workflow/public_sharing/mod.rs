//! Window-owned explicit public-copy admission and foreground clipboard delivery.
use std::io::Read;
use std::path::{Path, PathBuf};
use std::os::unix::fs::MetadataExt;
use std::sync::{Arc, Mutex, atomic::{AtomicBool, Ordering}};
use std::time::Instant;

use crate::connect::hosted::PreparedLink;

trait ReadyLink: Send {
    fn authorized(&self) -> bool;
    fn text(&self) -> &str;
}
impl ReadyLink for PreparedLink {
    fn authorized(&self) -> bool { self.authorized() }
    fn text(&self) -> &str { &self.text }
}

const LEASE: u64 = 30_000;

struct Entry {
    id: String,
    capture: String,
    bundle: PathBuf,
    inode: (u64, u64),
    generation: u64,
    expires: Instant,
    active: Arc<AtomicBool>,
    prepared: Option<Box<dyn ReadyLink>>,
    state: &'static str,
}

#[derive(Clone, Default)]
pub(crate) struct Controller(Arc<Mutex<Option<Entry>>>);

impl Controller {
    pub(crate) fn start(&self, capture: String, bundle: PathBuf, generation: u64) -> Result<String, &'static str> {
        if !crate::shared::handoff::experiment(crate::connect::hosted::HOSTED_SHARING) { return Err("feature_disabled"); }
        if !crate::connect::hosted::sharing_connected() { return Err("connect_required"); }
        let metadata = std::fs::metadata(&bundle).map_err(|_| "invalid_capture")?;
        let inode = (metadata.dev(), metadata.ino());
        let id = operation_id().map_err(|_| "local_state_error")?;
        let active = Arc::new(AtomicBool::new(true));
        {
            let mut guard = self.0.lock().map_err(|_| "app_busy")?;
            if let Some(old) = guard.take() { old.active.store(false, Ordering::SeqCst); }
            *guard = Some(Entry { id: id.clone(), capture, bundle: bundle.clone(), inode, generation, expires: Instant::now() + std::time::Duration::from_millis(LEASE), active: active.clone(), prepared: None, state: "preparing" });
        }
        let owner = self.clone();
        let operation = id.clone();
        std::thread::Builder::new().name("apparent-public-copy".into()).spawn(move || {
            let allowed = || active.load(Ordering::SeqCst) && owner.live(&id, generation);
            let result = crate::connect::hosted::prepare_public_link_handoff(&bundle, &allowed);
            let still_allowed = allowed();
            if let Ok(mut guard) = owner.0.lock() {
                if let Some(entry) = guard.as_mut().filter(|e| e.id == id && e.generation == generation && e.active.load(Ordering::SeqCst) && Instant::now() < e.expires) {
                    match result {
                        Ok(link) if still_allowed && link.authorized() => { entry.prepared = Some(Box::new(link)); entry.state = "prepared"; }
                        Ok(_) => { entry.state = "cancelled"; entry.active.store(false, Ordering::SeqCst); }
                        Err(_) => { entry.state = "failed"; entry.active.store(false, Ordering::SeqCst); }
                    }
                }
            }
        }).map_err(|_| { self.cancel_all(); "local_state_error" })?;
        Ok(operation)
    }

    fn live(&self, id: &str, generation: u64) -> bool {
        self.0.lock().ok().and_then(|mut guard| {
            guard.as_mut().map(|entry| {
                if Instant::now() >= entry.expires { entry.active.store(false, Ordering::SeqCst); entry.state = "expired"; }
                same_bundle(entry) && entry.id == id && entry.generation == generation && entry.active.load(Ordering::SeqCst) && entry.state == "preparing"
            })
        }).unwrap_or(false)
    }

    pub(crate) fn read(&self, id: &str, capture: &str, bundle: &Path, generation: u64) -> Result<&'static str, &'static str> {
        let mut guard = self.0.lock().map_err(|_| "app_busy")?;
        let entry = guard.as_mut().filter(|e| e.id == id).ok_or("no_operation")?;
        if Instant::now() >= entry.expires { entry.active.store(false, Ordering::SeqCst); entry.state = "expired"; }
        if entry.capture != capture || entry.bundle != bundle || !same_bundle(entry) || entry.generation != generation { entry.active.store(false, Ordering::SeqCst); entry.state = "cancelled"; return Err("conflict"); }
        Ok(entry.state)
    }

    pub(crate) fn cancel_all(&self) {
        if let Ok(mut guard) = self.0.lock() {
            if let Some(entry) = guard.as_mut() {
                entry.active.store(false, Ordering::SeqCst);
                entry.prepared = None;
                entry.state = "cancelled";
            }
        }
    }

    pub(crate) fn cancel(&self, id: &str) {
        if let Ok(mut guard) = self.0.lock() {
            if let Some(entry) = guard.as_mut().filter(|entry| entry.id == id) {
                entry.active.store(false, Ordering::SeqCst);
                entry.prepared = None;
                entry.state = "cancelled";
            }
        }
    }

    pub(crate) fn deliver(&self, id: &str, capture: &str, bundle: &Path, generation: u64, copy: impl FnOnce(&str) -> anyhow::Result<()>) -> Result<&'static str, &'static str> {
        let mut guard = self.0.lock().map_err(|_| "app_busy")?;
        let entry = guard.as_mut().filter(|e| e.id == id).ok_or("no_operation")?;
        if Instant::now() >= entry.expires { entry.active.store(false, Ordering::SeqCst); entry.state = "expired"; }
        if entry.capture != capture || entry.bundle != bundle || !same_bundle(entry) || entry.generation != generation || !entry.active.load(Ordering::SeqCst) { return Err("conflict"); }
        let link = entry.prepared.as_ref().ok_or("not_prepared")?;
        if !link.authorized() { entry.active.store(false, Ordering::SeqCst); entry.state = "cancelled"; return Err("cancelled"); }
        let copied = copy(link.text());
        entry.active.store(false, Ordering::SeqCst);
        if copied.is_err() { entry.state = "failed"; return Err("clipboard_failed"); }
        entry.state = "copied";
        entry.active.store(false, Ordering::SeqCst);
        Ok("copied")
    }
}

fn same_bundle(entry: &Entry) -> bool {
    std::fs::metadata(&entry.bundle).is_ok_and(|m| (m.dev(), m.ino()) == entry.inode)
}

fn operation_id() -> std::io::Result<String> {
    let mut bytes = [0u8; 16];
    std::fs::File::open("/dev/urandom")?.read_exact(&mut bytes)?;
    Ok(bytes.iter().map(|b| format!("{b:02x}")).collect())
}

#[cfg(test)]
mod tests {
 use super::*;
 struct Link(bool);
 impl ReadyLink for Link { fn authorized(&self)->bool{self.0} fn text(&self)->&str{"synthetic handoff"} }
 fn fixture()->(Controller,PathBuf) {
  let dir=std::env::temp_dir().join(format!("public-share-{}",operation_id().unwrap()));
  std::fs::create_dir(&dir).unwrap();
  let m=std::fs::metadata(&dir).unwrap();
  let c=Controller::default();
  *c.0.lock().unwrap()=Some(Entry{id:"a".repeat(32),capture:"same.apparent".into(),bundle:dir.clone(),inode:(m.dev(),m.ino()),generation:1,expires:Instant::now()+std::time::Duration::from_secs(10),active:Arc::new(AtomicBool::new(true)),prepared:Some(Box::new(Link(true))),state:"prepared"});
  (c,dir)
 }
 #[test]
 fn public_share_delivers_once_and_refuses_new_path_same_name() {
  let (c,dir)=fixture();let id="a".repeat(32);
  assert_eq!(c.deliver(&id,"same.apparent",&dir,1,|text|{assert_eq!(text,"synthetic handoff");Ok(())}),Ok("copied"));
  assert!(c.deliver(&id,"same.apparent",&dir,1,|_|panic!()).is_err());
  let(c2,dir2)=fixture();
  assert_eq!(c2.deliver(&id,"same.apparent",&dir,1,|_|panic!()),Err("conflict"));
  std::fs::remove_dir(dir).unwrap();std::fs::remove_dir(dir2).unwrap();
 }
 #[test]
 fn public_share_cancel_expiry_revoked_authority_and_clipboard_failure() {
  for fault in ["cancel","expired","revoked","clipboard","generation","inode"] {
   let(c,dir)=fixture();let id="a".repeat(32);
   match fault {
    "cancel"=>c.cancel(&id),
    "expired"=>c.0.lock().unwrap().as_mut().unwrap().expires=Instant::now(),
    "revoked"=>c.0.lock().unwrap().as_mut().unwrap().prepared=Some(Box::new(Link(false))),
    "inode"=>c.0.lock().unwrap().as_mut().unwrap().inode=(0,0),
    _=>()
   }
   let result=c.deliver(&id,"same.apparent",&dir,if fault=="generation"{2}else{1},|_|{assert_eq!(fault,"clipboard");anyhow::bail!("refused")});
   assert!(result.is_err());
   assert!(c.deliver(&id,"same.apparent",&dir,2,|_|panic!()).is_err());
   std::fs::remove_dir(dir).unwrap();
  }
 }
}
