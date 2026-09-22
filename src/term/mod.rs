//! Terminal motoru: PTY panelleri, bölme ağacı ve girdi kodlama.

pub mod input;
pub mod layout;
pub mod link;
pub mod pane;

use layout::{Node, PaneId};

/// Bir terminal sekmesi: bölme ağacı + odak + büyütme durumu.
#[derive(Clone, Debug)]
pub struct Tab {
    pub root: Node,
    pub focus: PaneId,
    pub zoomed: bool,
    /// Kullanıcının verdiği isim; yoksa odaktaki pane'den türetilir.
    pub name: Option<String>,
    /// Sekmenin açıldığı proje/dizin adı (otomatik başlık için).
    pub origin: String,
    /// Arka planda çıktı geldi mi (sekme şeridinde nokta).
    pub activity: bool,
    /// Dikkat istiyor: uzun komut bitti, zil çaldı ya da uygulama bildirim gönderdi.
    pub alert: bool,
}

impl Tab {
    pub fn new(pane: PaneId, origin: String) -> Tab {
        Tab { root: Node::Leaf(pane), focus: pane, zoomed: false, name: None, origin, activity: false, alert: false }
    }

    pub fn panes(&self) -> Vec<PaneId> {
        self.root.leaves()
    }
}
