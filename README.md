# Afterimage

Afterimage is a native macOS app that remembers what was on your screen. It takes a picture when you switch apps,
windows, spaces or tabs, when you open an app, and on a timer. Before a picture is saved, it finds the secrets in it
(passwords, keys, Social Security numbers, card and account numbers, QR setup codes) and frosts them over. What is
left is indexed so you can find it again by the words that were on screen, by a short description a local model
writes, or by meaning. The app is SwiftUI; the engine that hides, saves and searches is Rust.

## What it never does

- **It never contacts a cloud service.** The only model it talks to is LM Studio on the same Mac, over a loopback
  address, with proxies and redirects turned off. If you have set up LM Link, LM Studio may run the model on one of
  your own linked machines; the app names them in Settings.
- **Raw pixels never touch the disk.** A capture lives in memory until its secrets have been frosted. One code path
  writes pictures, and it is only handed the frosted frame. The same goes for text: the index, window titles and
  page addresses are masked first, and addresses lose their query and fragment.
- **It does not save when it is unsure.** By default a picture the model could not check is discarded, not saved.
  A browser whose windows cannot be read is left out of the picture. Signals from the browser extension can only
  add hiding, never remove it.

The frost is random noise tinted with averaged colours, not a blur, so it cannot be reversed.

## Requirements

- macOS 26 or later, on Apple silicon.
- [LM Studio](https://lmstudio.ai) with its local server on (port 1234 by default) and two models downloaded:
  - a model that can see pictures, used to find secrets and describe pictures. The default is
    `gemma-4-26b-a4b-it-qat-mlx`.
  - an embedding model, used for search by meaning. The default is `text-embedding-nomic-embed-text-v1.5`.

Both can be changed in Settings > Intelligence, as can the port. Search by words works without the embedding model.

## Install

1. Download the DMG from [Releases](https://github.com/jmaddington/afterimage/releases) and drag Afterimage to
   Applications. Releases are signed and notarized, and each comes with a `SHA256SUMS.txt`.
2. Start LM Studio. If its server is off, the welcome screen has a Start LM Studio button.
3. Open Afterimage and grant the two permissions it asks for:
   - **Screen Recording**, to take pictures. This one is required, and the app has to be reopened after you grant it.
   - **Accessibility**, to read page addresses so private windows and the sites you choose can be left out. On
     macOS 27 this setting is called "Device Control and Data Access".
4. Press Start.

Pictures and the index are kept in `~/Library/Application Support/Afterimage`, which is excluded from Time Machine
so that deleted pictures do not live on in backups.

## What it hides and what it leaves out

Hiding happens in layers, and a later layer can only add to what an earlier one found:

1. **Built-in rules** run first, on the text read from the screen. They catch card numbers, Social Security
   numbers, keys and tokens with known shapes, and labelled passwords. They do not depend on the model, so a model
   that is wrong or offline cannot undo them.
2. **The local model** reads the same text and reports credentials the rules cannot recognise, such as a password
   with no label. Anything it reports that cannot be located exactly hides the whole line.
3. **QR codes and barcodes** are frosted unless they hold nothing but a plain web address.
4. **The browser extension**, if connected, reports where password, card and account fields are on a page.

A secret found once is hidden everywhere it appears on that screen.

Some things are deliberately kept readable, because the point is to remember your day: people's names, phone
numbers, email and street addresses, dates, prices, order and tracking numbers, file paths, commit ids, and source
code that only handles secrets. In code, only a credential written out as a literal is hidden.

Some things are never in the picture at all (Settings > Privacy):

- password managers, along with their websites and browser extensions
- private browsing windows, and Tor Browser
- anything while a password is being typed
- browsers whose address cannot be read
- apps and sites you add. If a listed site is showing in any window of a browser, that whole browser is left out.

If the model is not available, the default is not to save. You can choose to fall back to the built-in rules
alone; they will not catch a password that has no label.

### Keeping and removing

Pictures are kept for 30 days and up to 20 GB by default, oldest removed first; both limits can be changed or
turned off. A picture you mark Keep is not removed automatically and does not count toward the limit. You can
delete pictures, or Forget the last 10 minutes, the last hour, today, or everything, which also drops captures
still waiting in memory.

## Browser extension

The extension tells the app where password, card and account fields are on a page, so that only those are frosted
and the rest of the page stays readable. When it can't say exactly where one is (inside an embedded frame, for
example), the whole browser is left out of that picture. It reports positions and kinds, never what is typed, and
only to the app on the same Mac.

It ships inside the app and is not yet in the browser stores:

1. Open Settings > Privacy > Browser Extension and press Connect next to your browser.
2. Press Show Extension to reveal the folder.
3. In the browser, open the extensions page, turn on Developer mode, choose Load unpacked and pick that folder.

Chrome, Edge, Brave, Vivaldi, Arc and Chromium are listed when installed, as is Firefox, which has its own folder.
The app works without the extension.

## Developer mode and agents

Developer mode (Settings > Intelligence) keeps a log of every request to LM Studio and its answer, and of each
decision to take a picture or not. Secrets are removed before anything is written, entries are kept for a week,
and turning the mode off deletes the log.

An MCP server for AI agents is in development and has not been released
([#3](https://github.com/jmaddington/afterimage/pull/3)). As proposed, it is off unless developer mode and its own
switch are both on, and it:

- is read-only: an agent can list a timeline, search, and read a saved picture and its text, and cannot change or
  delete anything;
- is local: it listens on a loopback address and refuses requests that come from a web page;
- requires a token on every request, which you can replace to cut off an agent.

An agent only sees what is already saved, with secrets already hidden. But if the agent runs in the cloud, what it
reads leaves your Mac.

## Building from source

You need Xcode 26, a stable Rust toolchain and Node.js. Packaging a DMG also needs `create-dmg`.

```sh
export DEVELOPER_DIR=/Applications/Xcode.app/Contents/Developer

cargo test -p afterimage-core                  # engine tests
./scripts/ffi.sh && (cd app && swift test)     # build the engine, generate Swift bindings, app tests
./scripts/extension.sh                         # extension tests and build
./scripts/build.sh                             # build/Afterimage.app
DMG_NO_FINDER=1 ./scripts/package.sh           # build/dist/Afterimage-<version>.dmg
```

`build.sh` signs with the first Developer ID in your keychain, or ad-hoc if there is none. macOS ties the two
permissions to the signature, so an ad-hoc build asks for them again after every build.

Tests that need a running LM Studio are skipped unless asked for:

```sh
cd app && AFTERIMAGE_LIVE=1 swift test --filter LiveModel
```

To try a build without touching your own library or settings, point it elsewhere with `AFTERIMAGE_HOME` (a folder)
and `AFTERIMAGE_DEFAULTS` (a defaults suite name).

## Status and limits

This is early software (v0.1.x). Treat it accordingly.

- Hiding is best effort. It depends on the text being read correctly from the screen and on a model's judgement.
  The rules and tests cover the cases that have been thought of; there has been no independent security review and
  there is no measured miss rate. A secret that is misread from the screen, or is in a form the rules and model do
  not recognise, can be saved.
- The library is not encrypted by the app. Frosted pictures and masked text are ordinary files in your user
  folder.
- Names, phone numbers, addresses and the contents of your work stay readable by design. If that is too much for a
  given app or site, add it to the list in Settings > Privacy.
- Every picture waits for a model check before it is saved, and the model has to be loaded in LM Studio for
  that. Descriptions are written only when plugged in by default.
- Running the model on a linked machine through LM Link has not been tried.
- There is no automatic update; new versions are on the Releases page.

## Licence

MIT (see [NOTICE](NOTICE)). Afterimage includes material adapted from
[Dayflow](https://github.com/JerryZLiu/Dayflow), which is MIT licensed; NOTICE lists what.
