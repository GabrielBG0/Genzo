# Raw test corpus (M0 step 0.C)

Raw files for the ProRAW and RAW decode spike. Everything in this folder except this README is gitignored: the files are large, and some are personal photos.

Run `python scripts/fetch-raw-corpus.py` to download the raw.pixls.us files and verify them against `manifest.json`.

Every raw.pixls.us file here must be **CC0**: the script looks up each file's license by sha256 in the site's own repository data, records it in the manifest, and refuses anything else. That matters because golden images derived from these files are committed. Most raw.pixls.us files are CC0, but not all (146 of about 2,000 were CC BY-NC-SA on 2026-10-18).

`golden-inputs.txt` lists the files the golden-image tests read. CI fetches only those (`--list corpus/golden-inputs.txt`) and caches them. Locally the tests skip when a file is missing, and print the command that fetches it.

## Layout

```
corpus/
  proraw/<iPhone model>/   Apple ProRAW DNGs, e.g. proraw/iPhone 15 Pro/IMG_0001.DNG
  raw/<maker>/             one file per camera maker, from raw.pixls.us
  reference/<file stem>/   reference renders of the same files (Adobe DNG Converter, Apple Photos)
```

## What 0.C needs in `proraw/`

For each iPhone generation available:

- a **48 MP ProRAW** file (iPhone 14 Pro and later)
- a **12 MP ProRAW** file (any ProRAW iPhone, iPhone 12 Pro and later)
- a **Bayer DNG**, if a third-party camera app made one

Use one scene per file with highlights, shadows and saturated colors, so the levels and matrices can be checked. Keep the originals: copy the files out of the Photos library or via AirDrop, not as exported JPEG or HEIC.
