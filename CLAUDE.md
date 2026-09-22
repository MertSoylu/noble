# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

# NOBLE — geliştirme notları

Rust (edition 2024, 1.88+) + ratatui ile yazılmış HUD terminal çalışma alanı. Özellikler, tuşlar ve
config şeması için `README.md`.

## Komutlar
- `cargo test` — birim + headless render + gerçek PTY + uçtan uca testler (hepsi geçmeli)
- `cargo test --test render` — yalnızca render/PTY testleri; `cargo test --test render <fn_adı>` tek test
- `cargo test --lib <modül_veya_test_adı>` — yalnızca `src/` içindeki birim testleri
- `cargo test --test e2e` — derlenmiş binary'yi sahte terminalde uçtan uca çalıştırır
- `cargo test --release --test e2e idle -- --ignored --nocapture` — boştaki CPU/RAM ölçümü (`#[ignore]`)
- `cargo test --release --test render heavy_output -- --ignored --nocapture` — 50 bin satır çıktı verimi
- `cargo clippy --all-targets` — uyarısız tutulur
- `cargo fmt` — `rustfmt.toml` (max_width 120)
- CI: `.github/workflows/ci.yml` (Windows: fmt + clippy `-D warnings` + tüm testler; Linux: clippy + `--lib`),
  `release.yml` (`v*` etiketinde Windows/Linux ikilileri)
- `cargo run -- --no-boot` — açılış animasyonu olmadan çalıştır; `--paths` config/veri konumlarını yazar
- `NOBLE_HOME` config ve veri dizinini taşır (gerçek config'i bozmadan denemek için)

## Kurallar
- **Her değişiklikten sonra uygulamayı güncelle:** `cmd /c install.cmd` çalıştır ki kullanıcı
  herhangi bir terminalde `noble` yazarak son sürümü açabilsin. Bu adım atlanmaz. (Betik, açık
  olan noble.exe'yi yeniden adlandırarak kurar; düz `cargo install` açıkken "Erişim engellendi" verir.)
- Kod içi yorumlar Türkçe, tanımlayıcılar ve arayüz metinleri İngilizce.
- UI değişikliğinden sonra `cargo test --test render` çalıştır ve `target/audit/*.txt`
  dökümlerini incele (taşma, hizalama, küçük boyutlar: 160×45 … 30×8).
- Çizim fonksiyonları yalnızca `ui/hud.rs` primitiflerini kullanır (sınır güvenli);
  `Buffer` üzerine doğrudan indeksle yazma.
- Durum yalnızca ana iş parçacığında değişir; arka plan işleri `AppEvent` gönderir.
- Ağ/CLI hataları asla panik değildir: sağlayıcılar `Err(String)` döner, UI "~" ile önbelleği gösterir.

## Mimari (birden çok dosyaya yayılan kısımlar)
- **lib + bin ayrımı:** tüm mantık `src/lib.rs` altındaki crate'te; `main.rs` yalnızca terminal
  kurulumu, panik güvenliği ve kare döngüsü. Testler `noble::app::App`'i doğrudan kurup
  `TestBackend` ile çizer — yeni durum/ekran eklerken testten erişilebilir kalmasına dikkat et.
- **Olay akışı:** giriş, sensör, proje tarama, git, AI toplayıcı ve her shell için okuyucu/bekleyici
  iş parçacıkları tek bir `mpsc` kanalına `event::AppEvent` yollar; `App` (`app/mod.rs`) bunları
  ana döngüde işler. Yeni arka plan işi = yeni `AppEvent` varyantı + `App` içinde eşleşen dal.
- **Yeniden çizim (pil dostu):** sabit kare hızı yok. `App::handle` olay görünür bir şeyi
  değiştirdiyse `true` döner (ör. terminaldeyken sensör olayı `false`); zamanla değişen her şey
  (saat, animasyon, yükleme göstergesi, bildirim süresi, imleç yanıp sönmesi) `App::redraw_after`'a
  eklenmeli, yoksa ekranda donar. Olay dışı değişiklikler `dirty`/`take_dirty` ile bildirilir.
  Çıktı altında ≤60 fps; boşta terminal dakikada bir, Home saniyede bir çizilir.
- **Arka plan işleri görünene göre:** sensörler `SensorMode` ile (System: süreç listesi dahil 1 sn,
  Home: 1 sn — pilde 2 sn, diğer: 5 sn), git durumu komut bitince / Home açılınca / değişen repo için,
  AI kotası yalnızca Home açıkken (`AiReq::Visible`, girişte hemen) istenir. Pilde (`App::on_battery`)
  Home saati saniyesiz ve sabit.
  Yeni periyodik iş eklerken yalnızca ilgili ekran açıkken çalıştır;
  `cargo test --release --test e2e idle -- --ignored --nocapture` ile ölç (dakika başına CPU ms).
- **Fare/hit-test:** `ui/*` çizim fonksiyonları çizerken `hits: Vec<(Rect, Hit)>` doldurur;
  `app/input.rs` tıklamayı bu listeden (en son eklenen önce) çözer. Tıklanabilir yeni öğe =
  `app::Hit` varyantı + çizimde `hits.push` + `input.rs`'de işleme. Hover vurgusu da bu listeye dayanır.
- **Terminal:** `term/layout.rs` saf split ağacı (PTY bilmez), `term/pane.rs` PTY + `vt100` +
  shell entegrasyonu (OSC 7 / OSC 9;9 ile cwd takibi), `term/input.rs` xterm tuş/fare kodlaması
  ve AltGr işleme.
- **Prompt sinyali:** `term/pane.rs` `Callbacks` her prompt'ta (OSC 7 / 9;9 / 133) `prompt`
  bayrağını kurar; `App::on_pty_output` bunu "komut bitti" sayar → o reponun git durumu
  `ProjectReq::Refresh` ile yenilenir, arka plan sekmesinde uzun komutsa bildirim (`notify`) çıkar.
  OSC 9 metni / OSC 777 ve zil de aynı yoldan sekmeye `alert` (◆) koyar.
- **Arama/bağlantı:** `app/search.rs` (geçmişte arama çubuğu, ctrl+tık), `term/link.rs`
  (URL ve `dosya:satır` tespiti). Eşleşmeler mutlak satır numarası tutar (0 = en eski geçmiş satırı).
- **Terminal renkleri:** `theme.rs` `TermScheme`/`TermPalette`; `wt.rs` Windows Terminal
  `settings.json`'ından (JSONC) PowerShell profilinin şemasını ve kullanıcı şemalarını okur →
  `App::term_schemes`. Pane çizimi (`ui/terminal.rs`) her karede `TermPalette::resolve` kullanır.
- **Claude hook'ları:** `hooks.rs` — Settings'ten açılınca `~/.claude/settings.json`'a
  `noble hook <olay>` ekler (yedek alır, yalnız kendi girdilerini kaldırır). `main.rs` bu alt komutu
  terminali açmadan işler; durum `data/agents/<NOBLE_INSTANCE>-<NOBLE_PANE>.json` dosyalarına yazılır,
  `App::tick` saniyede bir okur (`apply_hook_records` → `AgentState`). Testlerde gerçek dosyaya dokunma.
- **cmd.exe komutları:** komut argüman değil `NOBLE_LAUNCH` ortam değişkeniyle verilir
  (`cmd /K %NOBLE_LAUNCH%`); portable-pty'nin `\"` kaçışı cmd'de tırnaklı yolları bozar.
- **Kalıcılık:** `config.rs` canlı yeniden yüklenen `config.toml` (hata = toast, çökme yok);
  `store.rs` son dizinler, oturum, çalışma alanları, AI kullanım geçmişi ve arayüz durumunu
  (`state.json`: karşılama görüldü mü, sabitlenmiş projeler) atomik JSON yazımıyla saklar.

## Genişletme noktaları
- Yeni eylem: `src/keys.rs`'de `Action`'a ekle (+ `ALL`, `id`, `title`, `group`), `App::run`'da işle,
  istersen varsayılan tuş bağla.
- Yeni tema: `src/theme.rs`'deki `THEMES`'e ekle — Settings'te otomatik görünür.
- Yeni AI sağlayıcı: `src/ai/providers.rs`'de `detect`/`fetch` + `ProviderDef`, ayrıca payload testi.
  Token'lar yalnızca kendi sağlayıcısına gider; asla gösterilmez, loglanmaz veya yenilenmez.
