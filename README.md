# DeepL Client

Independent desktop translation client for **Windows x64** and **Linux x64**, built with Rust and Tauri 2. The interface supports English, German, Czech, French, Japanese and Italian. This project is not affiliated with or endorsed by DeepL or Microsoft.

## Features

- Translation through DeepL API Free/Pro or an experimental web provider.
- Write mode, contextual alternatives and dictionary lookup.
- Quick Assist with a searchable language menu, copy and edit actions.
- Local image OCR, screenshot area selection and file drag and drop.
- PDF, DOCX and PPTX translation with local document parsing.
- Microsoft Edge text-to-speech, history, favorites, tray controls and light/dark themes.

**Privacy:** OCR and document parsing run locally. Translation sends extracted text to the selected provider; speech sends text to Microsoft. Alternatives/dictionary features use DeepL's interactive web service even when API translation is selected. See [data handling](docs/PRIVACY.md).

## Getting started

Download the Windows EXE or Linux AppImage from [Releases](https://github.com/SebastianK90/DeepL-Client/releases), or build from source using the [Windows and Linux instructions](docs/BUILD.md). Windows builds use MSVC and embed the WebView2 loader and C runtime; the **Microsoft Edge WebView2 Runtime** must still be installed on the system.

**The app starts in the tray by default.** Open the main window from the tray menu. Disable **Start in tray** in Settings to show the main window on future launches. The preference takes effect on the next application start.

Choose a provider in Settings. Official API translation requires your own DeepL API key, stored in the operating system's credential store. The experimental web provider requires no API key but uses an undocumented service interface and may stop working. Write availability depends on the selected service and account permissions.

Quick Assist shortcuts:

| Desktop | Shortcut and behavior |
| --- | --- |
| Windows / Linux X11 | Select text, hold Ctrl and press C twice. Uses clipboard text. |
| KDE Plasma / Wayland | Select text and press the shortcut approved by the desktop portal, suggested **Ctrl+Alt+D**. Reads the PRIMARY selection; missing selection produces a message. |

Wayland behavior depends on the compositor, portal and source application's selection support. On Windows and KDE Plasma, left-clicking the tray icon opens the main window; double-clicking also works. Right-click for the tray menu. Closing the main window hides it in the tray; use the tray's Quit action to exit.

## Document limitations

PDF translation preserves independently positioned cells and keeps numeric values in their original columns. It does not implement complete table reflow. Long translations are fitted within the original space or rejected if they cannot fit readably. Scanned-image PDFs and nested Form XObjects are not supported by the PDF translation path; signed PDFs are rejected. Image OCR is available separately. Office layout also depends on the fonts and application used to open the result.

## Development

See [build and verification](docs/BUILD.md), [architecture](docs/ARCHITECTURE.md), [contributing](CONTRIBUTING.md) and [security reporting](SECURITY.md). CI builds Windows and Linux artifacts without publishing a release. Native Windows GUI checks are separate from Wine-based runtime checks.

Local build caches, user data, private documents, screenshots, historical audit evidence and proprietary application analysis excerpts are excluded from the source distribution.

## License

Original project code is licensed under the [MIT License](LICENSE). Third-party code, fonts and runtime components retain their own licenses; see [third-party notices](docs/THIRD_PARTY.md). DeepL and Microsoft services remain subject to their own terms.
