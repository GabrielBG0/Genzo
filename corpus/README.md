# Raw test corpus (M0 step 0.C)

Raw files for the ProRAW and RAW decode spike. Everything in this folder except this README is gitignored: the files are large, and some are personal photos.

Run `python scripts/fetch-raw-corpus.py` to download the raw.pixls.us files and verify them against `manifest.json`.

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
