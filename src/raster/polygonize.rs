use std::ptr;

use gdal_sys::CPLErr;

use crate::cpl::CslStringList;
use crate::errors::*;
use crate::raster::RasterBand;
use crate::utils::_last_cpl_err;
use crate::vector::LayerAccess;

/// Options that specify how to polygonize a raster band.
#[derive(Clone, Debug)]
pub struct PolygonizeOptions {
    /// Use 8 connectedness (diagonal pixels are considered connected).
    ///
    /// If `false` (default), 4 connectedness is used.
    pub eight_connected: bool,

    /// Name of a dataset from which to read the geotransform.
    ///
    /// This is useful if `band` has no related dataset, which is typical for mask bands.
    ///
    /// Corresponds to GDAL's `DATASET_FOR_GEOREF=dataset_name` option.
    pub dataset_for_georef: Option<String>,

    /// Interval in number of features at which transactions must be flushed.
    ///
    /// This option is supported by GDAL >= 3.12.
    ///
    /// - `0` means that no transactions are opened.
    /// - a negative value means a single transaction.
    ///
    /// Corresponds to GDAL's `COMMIT_INTERVAL=num` option.
    pub commit_interval: Option<i64>,
}

impl PolygonizeOptions {
    /// Create a polygonize options set.
    pub fn new() -> Self {
        Self {
            eight_connected: false,
            dataset_for_georef: None,
            commit_interval: None,
        }
    }

    /// Enable or disable 8-connectedness.
    ///
    /// If `state` is `true`, diagonal pixels are considered connected.
    /// Otherwise (default), 4-connectedness is used.
    pub fn with_eight_connected(&mut self, state: bool) -> &mut Self {
        self.eight_connected = state;
        self
    }

    /// Specify a dataset name from which to read the geotransform.
    ///
    /// This is useful if the source `band` has no related dataset, which is typical for mask
    /// bands.
    ///
    /// Corresponds to GDAL's `DATASET_FOR_GEOREF=dataset_name` option.
    pub fn with_dataset_for_georef<S: Into<String>>(&mut self, dataset_name: S) -> &mut Self {
        self.dataset_for_georef = Some(dataset_name.into());
        self
    }

    /// Specify the interval (in number of features) at which transactions must be flushed.
    ///
    /// This option is supported by GDAL >= 3.12.
    ///
    /// - `0` means that no transactions are opened.
    /// - a negative value means a single transaction.
    ///
    /// Corresponds to GDAL's `COMMIT_INTERVAL=num` option.
    pub fn with_commit_interval(&mut self, interval: i64) -> &mut Self {
        self.commit_interval = Some(interval);
        self
    }

    /// Render relevant options into [`CslStringList`] values, as compatible with
    /// [`gdal_sys::GDALPolygonize`].
    pub fn to_options_list(&self) -> Result<CslStringList> {
        let mut options = CslStringList::new();

        // Per GDAL docs and implementation, the presence of 8CONNECTED enables 8-connectedness.
        // The canonical value is "8".
        if self.eight_connected {
            options.set_name_value("8CONNECTED", "8")?;
        }

        if let Some(dataset_for_georef) = self.dataset_for_georef.as_deref() {
            options.set_name_value("DATASET_FOR_GEOREF", dataset_for_georef)?;
        }

        if let Some(commit_interval) = self.commit_interval {
            options.set_name_value("COMMIT_INTERVAL", &commit_interval.to_string())?;
        }

        Ok(options)
    }
}

/// Convert a raster band into polygons.
///
/// This is a safe wrapper around GDAL's `GDALPolygonize`.
///
/// - `band` is the source raster band.
/// - `mask_band` (optional) restricts processing to pixels where mask is non-zero.
/// - `layer` is the output vector layer. It must already exist and be suitable for polygon output.
/// - `field_index` is the field index (0-based) to store the pixel value, or `-1` to skip.
/// - `options` controls algorithm behavior.
pub fn polygonize<L: LayerAccess>(
    band: &RasterBand<'_>,
    mask_band: Option<&RasterBand<'_>>,
    layer: &L,
    field_index: i32,
    options: &PolygonizeOptions,
) -> Result<()> {
    let c_options = options.to_options_list()?;

    // SAFETY: We only pass handles obtained from safe `gdal` wrapper objects.
    // GDALPolygonize does not take ownership of the band or layer handles.
    let rv = unsafe {
        gdal_sys::GDALPolygonize(
            band.c_rasterband(),
            mask_band
                .map(|b| b.c_rasterband())
                .unwrap_or_else(ptr::null_mut),
            layer.c_layer(),
            field_index,
            c_options.as_ptr(),
            None,
            ptr::null_mut(),
        )
    };

    if rv != CPLErr::CE_None {
        return Err(_last_cpl_err(rv));
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use crate::raster::ByteBuffer;
    use crate::vector::LayerAccess;
    use crate::vector::LayerOptions;
    use crate::vsi::unlink_mem_file;
    use crate::DriverManager;
    use gdal_sys::{OGRFieldType, OGRwkbGeometryType};

    use super::PolygonizeOptions;

    #[test]
    fn test_polygonizeoptions_as_ptr() {
        let c_options = PolygonizeOptions::new().to_options_list().unwrap();
        assert_eq!(c_options.fetch_name_value("8CONNECTED"), None);
        assert_eq!(c_options.fetch_name_value("DATASET_FOR_GEOREF"), None);
        assert_eq!(c_options.fetch_name_value("COMMIT_INTERVAL"), None);

        let c_options = PolygonizeOptions {
            eight_connected: true,
            dataset_for_georef: Some("/vsimem/georef.tif".to_string()),
            commit_interval: Some(12345),
        }
        .to_options_list()
        .unwrap();
        assert_eq!(c_options.fetch_name_value("8CONNECTED"), Some("8".into()));
        assert_eq!(
            c_options.fetch_name_value("DATASET_FOR_GEOREF"),
            Some("/vsimem/georef.tif".into())
        );
        assert_eq!(
            c_options.fetch_name_value("COMMIT_INTERVAL"),
            Some("12345".into())
        );
    }

    #[test]
    fn test_polygonize_connectivity_affects_regions() {
        // 3x3 raster with diagonal 1s:
        // 1 0 0
        // 0 1 0
        // 0 0 1
        // Under 4-connectedness the value=1 pixels are 3 separate regions.
        // Under 8-connectedness they are one connected region.

        let mem_driver = DriverManager::get_driver_by_name("MEM").unwrap();
        let raster_ds = mem_driver.create("", 3, 3, 1).unwrap();
        let mut band = raster_ds.rasterband(1).unwrap();

        let mut data = ByteBuffer::new((3, 3), vec![1u8, 0, 0, 0, 1, 0, 0, 0, 1]);
        band.write((0, 0), (3, 3), &mut data).unwrap();

        let gpkg_path = "/vsimem/test_polygonize_connectivity.gpkg";
        let gpkg_driver = DriverManager::get_driver_by_name("GPKG").unwrap();
        let mut vector_ds = gpkg_driver.create_vector_only(gpkg_path).unwrap();

        // 4-connected output
        let mut layer_4 = vector_ds
            .create_layer(LayerOptions {
                name: "four",
                ty: OGRwkbGeometryType::wkbPolygon,
                ..Default::default()
            })
            .unwrap();
        layer_4
            .create_defn_fields(&[("val", OGRFieldType::OFTInteger)])
            .unwrap();

        super::polygonize(&band, None, &layer_4, 0, &PolygonizeOptions::new()).unwrap();

        let ones_4 = layer_4
            .features()
            .filter_map(|f| f.field_as_integer(0).unwrap())
            .filter(|v| *v == 1)
            .count();
        assert_eq!(ones_4, 3);

        // 8-connected output
        let mut layer_8 = vector_ds
            .create_layer(LayerOptions {
                name: "eight",
                ty: OGRwkbGeometryType::wkbPolygon,
                ..Default::default()
            })
            .unwrap();
        layer_8
            .create_defn_fields(&[("val", OGRFieldType::OFTInteger)])
            .unwrap();

        super::polygonize(
            &band,
            None,
            &layer_8,
            0,
            &PolygonizeOptions {
                eight_connected: true,
                dataset_for_georef: None,
                commit_interval: None,
            },
        )
        .unwrap();

        let ones_8 = layer_8
            .features()
            .filter_map(|f| f.field_as_integer(0).unwrap())
            .filter(|v| *v == 1)
            .count();
        assert_eq!(ones_8, 1);

        unlink_mem_file(gpkg_path).unwrap();
    }

    #[test]
    fn test_polygonize_with_mask_band_restricts_output() {
        let mem_driver = DriverManager::get_driver_by_name("MEM").unwrap();
        let raster_ds = mem_driver.create("", 3, 3, 2).unwrap();

        let mut value_band = raster_ds.rasterband(1).unwrap();
        let mut mask_band = raster_ds.rasterband(2).unwrap();

        // Value band: all 7s.
        let mut values = ByteBuffer::new((3, 3), vec![7u8; 9]);
        value_band.write((0, 0), (3, 3), &mut values).unwrap();

        // Mask: only the center pixel is included.
        let mut mask = ByteBuffer::new((3, 3), vec![0u8, 0, 0, 0, 1, 0, 0, 0, 0]);
        mask_band.write((0, 0), (3, 3), &mut mask).unwrap();

        let gpkg_path = "/vsimem/test_polygonize_mask.gpkg";
        let gpkg_driver = DriverManager::get_driver_by_name("GPKG").unwrap();
        let mut vector_ds = gpkg_driver.create_vector_only(gpkg_path).unwrap();

        let mut layer = vector_ds
            .create_layer(LayerOptions {
                name: "masked",
                ty: OGRwkbGeometryType::wkbPolygon,
                ..Default::default()
            })
            .unwrap();
        layer
            .create_defn_fields(&[("val", OGRFieldType::OFTInteger)])
            .unwrap();

        super::polygonize(
            &value_band,
            Some(&mask_band),
            &layer,
            0,
            &PolygonizeOptions::new(),
        )
        .unwrap();

        assert_eq!(layer.feature_count(), 1);
        let only_val = layer
            .features()
            .next()
            .unwrap()
            .field_as_integer(0)
            .unwrap();
        assert_eq!(only_val, Some(7));

        unlink_mem_file(gpkg_path).unwrap();
    }
}
