# Vendored libraries

Each library sits in `<name>-<version>/` with its license file. Files were
downloaded from the jsDelivr npm mirror on 2026-10-26; the page loads them
from these local copies only.

| Library | Version | License | File | Source URL | sha256 |
|---|---|---|---|---|---|
| Leaflet | 1.9.4 | BSD-2-Clause | `leaflet.js` | https://cdn.jsdelivr.net/npm/leaflet@1.9.4/dist/leaflet.js | `db49d009c841f5ca34a888c96511ae936fd9f5533e90d8b2c4d57596f4e5641a` |
| Leaflet | 1.9.4 | BSD-2-Clause | `leaflet.css` | https://cdn.jsdelivr.net/npm/leaflet@1.9.4/dist/leaflet.css | `a7837102824184820dfa198d1ebcd109ff6d0ff9a2672a074b9a1b4d147d04c6` |
| Leaflet | 1.9.4 | BSD-2-Clause | `images/*.png` | https://cdn.jsdelivr.net/npm/leaflet@1.9.4/dist/images/ | |
| proj4js | 2.22.0 | MIT | `proj4.js` | https://cdn.jsdelivr.net/npm/proj4@2.22.0/dist/proj4.js | `af7df653d91ea591f33d26fb958990bbd3071b2db644a4edaba441cc9861a474` |
| geotiff.js | 3.0.5 | MIT | `geotiff.js` | https://cdn.jsdelivr.net/npm/geotiff@3.0.5/dist-browser/geotiff.js | `8cfc01ffc945904766d8fb89f19da7afb799e342834454f5592fbcf0bcfe2db5` |
| georaster | 1.6.0 | Apache-2.0 | `georaster.browser.bundle.min.js` | https://cdn.jsdelivr.net/npm/georaster@1.6.0/dist/georaster.browser.bundle.min.js | `b782c68192378940ba83296b66090eaf523dc264b581d30fd68564c4f9cf1b81` |
| georaster-layer-for-leaflet | 4.1.2 | Apache-2.0 | `georaster-layer-for-leaflet.min.js` | https://cdn.jsdelivr.net/npm/georaster-layer-for-leaflet@4.1.2/dist/v3/webpack/bundle/georaster-layer-for-leaflet.min.js | `ef197f4564912fe4c7a7421a5f3b267c9dd65333e4688188821c7b2f8303c0f5` |
| PMTiles | 4.5.0 | BSD-3-Clause | `pmtiles.js` | https://cdn.jsdelivr.net/npm/pmtiles@4.5.0/dist/pmtiles.js | `caf981bc46f6327ee7e65d5dc964d89d38a69f60edca2bd4c5c890c21b554c6c` |
| protomaps-leaflet | 5.1.0 | BSD-3-Clause | `protomaps-leaflet.js` | https://cdn.jsdelivr.net/npm/protomaps-leaflet@5.1.0/dist/protomaps-leaflet.js | `26af014f7b1af308ec120b791cff76657bb9c3383633b52033e6edf9e5e4cdb5` |

License sources: each package's own `LICENSE` (`LICENSE.md` for proj4) at the
same jsDelivr version path, except PMTiles, whose npm package ships none; its
`LICENSE` is https://raw.githubusercontent.com/protomaps/PMTiles/main/LICENSE.
`georaster-layer-for-leaflet.min.js.LICENSE.txt` lists the licenses of the
packages bundled into that file.

Declared dependencies (from each `package.json`): georaster 1.6.0 depends on
geotiff `1.0.0-beta.13`; the page decodes GeoTIFFs with geotiff.js 3.0.5 and
passes the pixel arrays to georaster. georaster-layer-for-leaflet 4.1.2
depends on `proj4-collect`, which merges its own proj4 and
proj4-fully-loaded with the global `proj4` defs. protomaps-leaflet 5.1.0
depends on pmtiles `^3.1.0`; the page passes it a `PMTiles` instance from
pmtiles 4.5.0.
