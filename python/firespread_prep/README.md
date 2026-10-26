# firespread-prep

Data preparation for the Hermits Peak / Calf Canyon 2022 case. Every script
runs as `python -m firespread_prep.<name>` from the `firespread` venv. Each
one reads and writes under the repository's gitignored `data/` directory.
Set `FIRESPREAD_DATA` to use another directory. Install with
`pip install -e python/firespread_prep`.

## extent

Unions the two WFIGS final perimeters in `data/wfigs/`. Prints their bounding
box in EPSG:4326 and in EPSG:5070. Writes `data/extent.json` with the master
grid. The grid is a 2334 by 2334 window of 30 m EPSG:5070 cells. The window
is centred on the union's centroid, with its corners snapped to multiples of
30 m. `origin_x` and `origin_y` give the upper-left corner. `bounds_5070` and
the grid's `bbox_4326` describe the same window. `covers_wfigs_union` says
whether the window contains the WFIGS union's bounding box.

## feds_filter

Reads the FEDS 2022 large-fire perimeter file and new-fire-pixel file. Both
are FlatGeobuf in EPSG:9311. The bounding-box filter is the grid window
padded by 0.1 degree and reprojected into EPSG:9311. Writes every matching
row to `data/feds/hermits_perimeters.gpkg` and
`data/feds/hermits_newfirepix.gpkg`. Writes one summary per `fireID` to
`data/feds/hermits_fireids.json` and `data/feds/hermits_newfirepix_fireids.json`.
A summary has the first and last `t`, the timestep count and the max `farea`.
The perimeter JSON also lists, for each timestep from 2022-04-22 on, the
largest `fireID` and its share of the summed `farea`.

## windows

Writes `data/windows.json`. There are three label pairs: 2022-04-22 12:00 to
04-23 00:00, 05-01 12:00 to 05-02 00:00, and 05-10 12:00 to 05-11 12:00. For
each pair the script finds the nearest FEDS perimeter timesteps. It records
the perimeter rows of every `fireID` present at both timesteps. It records the
new-fire-pixel rows between them. It lists the whole UTC hours from the start
to the target, inclusive. `overpass_times_utc` is null, because the FEDS
new-fire-pixel file has no per-detection timestamp.

## hrrr_subset

Runs over every hour in `data/windows.json`. For each hour it fetches the
`.idx` inventory of `hrrr.YYYYMMDD/conus/hrrr.tHHz.wrfsfcf00.grib2` from the
NOAA S3 bucket. It locates six records by their `VAR:LEVEL` strings: TMP 2 m,
RH 2 m, UGRD 10 m, VGRD 10 m, GUST surface and TCDC entire atmosphere. It
fetches each record with one HTTP Range request. It writes
`data/hrrr/hrrr.YYYYMMDD.tHHz.f00.<tag>.grib2`. Existing files are kept. A
record number that differs from the NCEP inventory (71, 75, 77, 78, 9, 116)
is logged. At the end it opens one fragment with rasterio
and prints its CRS and shape.

## gridmet_slice

Reads `data/gridmet/fm100_2022.nc` and `fm1000_2022.nc` with rasterio's
netCDF driver. For each calendar date spanned by `data/windows.json` it
selects the band whose `NETCDF_DIM_day` matches the date. It clips the band
to the grid window padded by one cell. It applies the 0.1 scale factor and
masks 32767 to NaN. It writes `data/gridmet/<product>_<date>.tif` as Float32
EPSG:4326 at the native 1/24 degree resolution. `--inspect` prints the netCDF
structure instead.

## lfps_submit

`submit` builds the LANDFIRE Product Service request. The layer list is
FBFM40, CC, CH, CBH, CBD and FDist from LF2022, and Elev, SlpD and Asp from
LF2020. The area of interest is the grid window's WGS84 bounding box. The
output projection is 5070. The resolution is the native 30 m, so
`Resample_Resolution` is omitted unless `--resample` is given, because the
service accepts 31 to 9999 only. `--dry-run` prints the URL and exits.
Otherwise `--email` is required, and the response is saved to
`data/landfire/lfps_job.json`. `poll` queries `/api/job/status` until a
terminal status. It then downloads the product zip to
`data/landfire/<jobId>.zip`.

## align

Unzips an LFPS product zip into `data/landfire/<zip stem>/`. Warps each band
of its multiband GeoTIFF onto the `data/extent.json` grid. FBFM40, FDist and
Asp use nearest resampling. The other layers use bilinear. CBH and CH are
divided by 10, and CBD by 100. Writes one Float32 Deflate GeoTIFF per layer to
`data/grid/<layer>.tif` with nodata -9999. `tests/test_align.py` runs it on a
synthetic 3 by 3 raster. Run it with
`python -m unittest discover -s python/firespread_prep/tests`.

## inspect_feds

Prints the CRS, fields, dtypes, feature count and three rows of each FEDS
FlatGeobuf file.
