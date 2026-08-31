# kvittokartan

BankID → kvitton → kategorisering → en lokal HTML-dashboard, med en
Uppdatera-knapp per (person, kedja) som kör igenom hela flödet direkt från
vilken enhet som helst som tittar på sidan.

Källorna lever i egna repon, inbundna som git-submoduler:

- [`kvitto-core`](../kvitto-core) — delad modell, lagring, kategorisering, jobbmotorn bakom Uppdatera-knappen. Kedjeagnostisk.
- [`kvitto-ica`](../kvitto-ica) — ICA-kvitton via Kivra (BankID OAuth2/PKCE).
- [`kvitto-willys`](../kvitto-willys) — Willys-kvitton via BankID (se den crate:ns egen README för varför en bakgrundswebbläsare behövs där, men inte här).
- Hemköp: inte implementerad än.

## Kom igång

```sh
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
- Inget personnummer skickas någonsin till ICA/Kivra eller Willys av det
  här verktyget — BankID-signaturen är hela identiteten.

## Kategori-overrides: nyckelformatet har ändrats

Om du migrerar från den fristående `ica-sync`: dess
`data/category_overrides.json` nycklas på ett rått, normaliserat varunamn.
Här nycklas overrides istället `"{kedja}:{artikelnummer eller
~normaliserat namn}"` (t.ex. `willys:101233933_ST` eller `ica:~kvarg
vanilj`) — annars skulle ICA:s och Willys artikelnummer kunna kollidera med
tiden och tyst felkategorisera nåt. En gammal `category_overrides.json`
funkar inte rakt av här; skriv en liten engångsmigrering om du har
overrides värda att bevara, annars är det bara att kategorisera om de
varorna igen via dashboarden.
