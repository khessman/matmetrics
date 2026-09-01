# matmetrics

BankID → kvitton → kategorisering → en lokal HTML-dashboard, med en
Uppdatera-knapp per (person, kedja) som kör igenom hela flödet direkt från
vilken enhet som helst som tittar på sidan.

**Stödda kedjor just nu: ICA, Willys och Hemköp.** Willys och Hemköp är
samma Axfood/Hybris-backend bakom olika domäner — samma inloggnings- och
kvittoformat, en gemensam crate. ICA går via Kivra med standard OAuth2/PKCE.

## Arkitektur

Detta repo (`matmetrics`) är huvudprojektet: workspace-`Cargo.toml`,
`serve`-binären, dashboard-templaten (`report_template.html`) och
kategoriseringsregler. Källorna för varje kedja lever i egna repon, inbundna
här som git-submoduler under `crates/`:

- [`kvitto-core`](https://github.com/khessman/kvitto-core) — delad modell,
  lagring, kategorisering, jobbmotorn bakom Uppdatera-knappen.
  Kedjeagnostisk, ingen HTTP-klient här.
- [`kvitto-ica`](https://github.com/khessman/kvitto-ica) — ICA-kvitton via
  Kivra (BankID OAuth2/PKCE).
- [`kvitto-willys`](https://github.com/khessman/kvitto-willys) — Willys- och
  Hemköp-kvitton via BankID. `collect-login` blockeras av ett WAF-skydd mot
  rena HTTP-klienter (och mot headless webbläsare), så inloggningen körs
  genom en riktig, synlig Chromium-instans — se crate:ns källkodskommentarer
  för detaljerna.

Att lägga till en ny kedja är: skriv en ny crate som implementerar
`ReceiptSource` (traiten i `kvitto-core`), lägg till den i
`AppState::sources()` i `src/serve_sync.rs`, och en rad i `SOURCE_ROWS` i
`src/report.rs`. Dashboarden, kategoriseraren och profilhanteringen rör man
inte.

## Kom igång

```sh
git clone https://github.com/khessman/matmetrics.git
cd matmetrics
git submodule update --init
cargo build --release
cargo run -- serve
```

Öppna `http://localhost:7878/report.html`. Utan `config.toml` körs allt mot
en enda profil ("du"). Kopiera `config.example.toml` → `config.toml` för
fler hushållsmedlemmar — se den filen för formatet.

## Kommandon

| Kommando | Gör |
|---|---|
| `serve [--port]` | Servar dashboarden, en Uppdatera-knapp per (profil, källa) |
| `report` | Bygger `out/report.html` från redan synkade kvitton, ingen inloggning |
| `reparse [--force]` | Kör om parser + kategorisering mot arkivet, ingen inloggning |

## Säkerhet & integritet

- Tokens/cookies/sessioner skrivs aldrig till disk — bara i minnet, hela
  processens livstid. En omstart kräver ny BankID-inloggning; det är
  medveten friktion, inte ett förbiseende.
- `serve` bindar `0.0.0.0` **utan autentisering** — kvittodata avslöjar
  exakt vad och när någon handlat. Exponera inte utanför det lokala
  nätverket.
- Inget personnummer skickas någonsin till ICA/Kivra, Willys eller Hemköp av
  det här verktyget — BankID-signaturen är hela identiteten.
- Riktiga kvitton (inklusive medlemsnummer, butik, köphistorik) hör aldrig
  hemma i git, inte ens redigerade — se `.gitignore` i `kvitto-willys` för
  testfixturer som medvetet hålls utanför versionshantering.

## Kategori-overrides: nyckelformatet har ändrats

Om du migrerar från den fristående `ica-sync`: dess
`data/category_overrides.json` nycklas på ett rått, normaliserat varunamn.
Här nycklas overrides istället `"{kedja}:{artikelnummer eller
~normaliserat namn}"` (t.ex. `willys:101233933_ST` eller `ica:~kvarg
vanilj`) — annars skulle olika kedjors artikelnummer kunna kollidera med
tiden och tyst felkategorisera nåt. En gammal `category_overrides.json`
funkar inte rakt av här; skriv en liten engångsmigrering om du har
overrides värda att bevara, annars är det bara att kategorisera om de
varorna igen via dashboarden.

## Licens

MIT, se [`LICENSE`](LICENSE).
