//! GDAL VRT (Virtual Raster) API
//!
//! This module provides safe wrappers for creating and manipulating VRT (Virtual Raster) datasets.
//! VRTs are a powerful GDAL feature that allow creating virtual datasets that reference
//! other raster data sources without copying the data.
//!
//! ## Example
//!
//! ```rust, no_run
//! use gdal::Dataset;
//! use gdal::vrt::VrtDataset;
//! use gdal::raster::GdalDataType;
//!
//! # fn main() -> gdal::errors::Result<()> {
//! // Create a new VRT dataset
//! let mut vrt = VrtDataset::create(100, 100)?;
//!
//! // Set geotransform
//! vrt.set_geo_transform(&[0.0, 1.0, 0.0, 100.0, 0.0, -1.0])?;
//!
//! // Add a band
//! vrt.add_band(GdalDataType::Float32, None)?;
//!
//! // Open a source dataset and add it as a simple source
//! let source = Dataset::open("source.tif")?;
//! let source_band = source.rasterband(1)?;
//!
//! let vrt_band = vrt.rasterband(1)?;
//! vrt_band.add_simple_source(
//!     &source_band,
//!     (0, 0, 100, 100),  // source window
//!     (0, 0, 100, 100),  // destination window
//!     None,              // resampling (default: nearest)
//!     None,              // nodata value
//! )?;
//! # Ok(())
//! # }
//! ```

use std::ffi::CString;
use std::ptr::null_mut;

use gdal_sys::{
    CPLErr, GDALAddBand, GDALClose, GDALDatasetH, GDALGetRasterBand, GDALRasterBandH,
    GDALSetGeoTransform, GDALSetProjection, VRTAddSimpleSource, VRTCreate, VRTDatasetH,
    VRTSourcedRasterBandH,
};

use crate::errors::{GdalError, Result};
use crate::raster::{GdalDataType, RasterBand};
use crate::utils::{_last_cpl_err, _last_null_pointer_err};
use crate::Dataset;

/// Special value indicating that nodata is not set for a VRT source.
/// This matches the `VRT_NODATA_UNSET` constant from GDAL's `gdal_vrt.h`.
pub const NODATA_UNSET: f64 = -1234.56;

/// A VRT (Virtual Raster) dataset.
///
/// VRT datasets are virtual datasets that can reference data from other raster sources.
/// They are useful for:
/// - Creating mosaics from multiple files
/// - Subsetting or resampling raster data
/// - Adding derived bands or applying transformations
///
/// When dropped, the VRT dataset is automatically closed.
pub struct VrtDataset {
    c_dataset: VRTDatasetH,
}

impl Drop for VrtDataset {
    fn drop(&mut self) {
        unsafe {
            GDALClose(self.c_dataset as GDALDatasetH);
        }
    }
}

// GDAL Docs state: The returned dataset should only be accessed by one thread at a time.
unsafe impl Send for VrtDataset {}

impl VrtDataset {
    /// Creates a new empty VRT dataset with the given dimensions.
    ///
    /// # Arguments
    /// * `x_size` - Width of the dataset in pixels
    /// * `y_size` - Height of the dataset in pixels
    ///
    /// # Returns
    /// A new `VrtDataset` on success.
    ///
    /// # Example
    /// ```rust, no_run
    /// use gdal::vrt::VrtDataset;
    ///
    /// # fn main() -> gdal::errors::Result<()> {
    /// let vrt = VrtDataset::create(1024, 768)?;
    /// # Ok(())
    /// # }
    /// ```
    pub fn create(x_size: usize, y_size: usize) -> Result<Self> {
        let c_dataset = unsafe { VRTCreate(x_size as i32, y_size as i32) };

        if c_dataset.is_null() {
            return Err(_last_null_pointer_err("VRTCreate"));
        }

        Ok(VrtDataset { c_dataset })
    }

    /// Returns the raw VRT dataset handle.
    ///
    /// # Safety
    /// The returned handle is only valid for the lifetime of this struct.
    /// Do not close or transfer ownership of the handle.
    pub fn c_dataset(&self) -> VRTDatasetH {
        self.c_dataset
    }

    /// Returns the VRT dataset as a GDAL dataset handle.
    ///
    /// This is useful for calling GDAL functions that expect a `GDALDatasetH`.
    pub fn as_gdal_dataset_handle(&self) -> GDALDatasetH {
        self.c_dataset as GDALDatasetH
    }

    /// Creates a GDAL `Dataset` wrapper for this VRT.
    ///
    /// # Safety
    /// The returned Dataset borrows from this VrtDataset and must not outlive it.
    /// The caller must ensure the Dataset is dropped before this VrtDataset.
    /// Do not call `GDALClose` on the returned dataset.
    pub unsafe fn as_dataset(&self) -> Dataset {
        Dataset::from_c_dataset(self.c_dataset as GDALDatasetH)
    }

    /// Sets the geotransform for the VRT dataset.
    ///
    /// # Arguments
    /// * `transform` - A 6-element array containing the geotransform coefficients:
    ///   `[origin_x, pixel_width, rotation_x, origin_y, rotation_y, pixel_height]`
    ///
    /// # Example
    /// ```rust, no_run
    /// use gdal::vrt::VrtDataset;
    ///
    /// # fn main() -> gdal::errors::Result<()> {
    /// let mut vrt = VrtDataset::create(100, 100)?;
    /// // Set origin at (0, 100), 1 unit per pixel, no rotation
    /// vrt.set_geo_transform(&[0.0, 1.0, 0.0, 100.0, 0.0, -1.0])?;
    /// # Ok(())
    /// # }
    /// ```
    pub fn set_geo_transform(&mut self, transform: &[f64; 6]) -> Result<()> {
        let mut transform_copy = *transform;
        let rv = unsafe {
            GDALSetGeoTransform(self.c_dataset as GDALDatasetH, transform_copy.as_mut_ptr())
        };

        if rv != CPLErr::CE_None {
            return Err(_last_cpl_err(rv));
        }
        Ok(())
    }

    /// Sets the spatial reference system (projection) for the VRT dataset.
    ///
    /// # Arguments
    /// * `projection` - A WKT string or other GDAL-supported projection specification
    ///
    /// # Example
    /// ```rust, no_run
    /// use gdal::vrt::VrtDataset;
    ///
    /// # fn main() -> gdal::errors::Result<()> {
    /// let mut vrt = VrtDataset::create(100, 100)?;
    /// vrt.set_projection("EPSG:4326")?;
    /// # Ok(())
    /// # }
    /// ```
    pub fn set_projection(&mut self, projection: &str) -> Result<()> {
        let c_projection = CString::new(projection)
            .map_err(|_| GdalError::BadArgument("Projection string contains null byte".into()))?;

        let rv =
            unsafe { GDALSetProjection(self.c_dataset as GDALDatasetH, c_projection.as_ptr()) };

        if rv != CPLErr::CE_None {
            return Err(_last_cpl_err(rv));
        }
        Ok(())
    }

    /// Adds a new band to the VRT dataset.
    ///
    /// # Arguments
    /// * `data_type` - The data type for the new band
    /// * `options` - Optional list of creation options (currently unused for VRT bands)
    ///
    /// # Returns
    /// The 1-based index of the newly created band on success.
    ///
    /// # Example
    /// ```rust, no_run
    /// use gdal::vrt::VrtDataset;
    /// use gdal::raster::GdalDataType;
    ///
    /// # fn main() -> gdal::errors::Result<()> {
    /// let mut vrt = VrtDataset::create(100, 100)?;
    /// let band_index = vrt.add_band(GdalDataType::Float32, None)?;
    /// assert_eq!(band_index, 1);
    /// # Ok(())
    /// # }
    /// ```
    pub fn add_band(
        &mut self,
        data_type: GdalDataType,
        _options: Option<&[&str]>,
    ) -> Result<usize> {
        let rv = unsafe {
            GDALAddBand(
                self.c_dataset as GDALDatasetH,
                data_type.gdal_ordinal(),
                null_mut(),
            )
        };

        if rv != CPLErr::CE_None {
            return Err(_last_cpl_err(rv));
        }

        // Return the band count (1-based index of the new band)
        Ok(self.raster_count())
    }

    /// Returns the number of raster bands in the VRT dataset.
    pub fn raster_count(&self) -> usize {
        (unsafe { gdal_sys::GDALGetRasterCount(self.c_dataset as GDALDatasetH) }) as usize
    }

    /// Fetches a band from the VRT dataset.
    ///
    /// # Arguments
    /// * `band_index` - The 1-based index of the band to fetch
    ///
    /// # Returns
    /// A `VrtRasterBand` wrapper for the band.
    pub fn rasterband(&self, band_index: usize) -> Result<VrtRasterBand<'_>> {
        let c_band =
            unsafe { GDALGetRasterBand(self.c_dataset as GDALDatasetH, band_index as i32) };

        if c_band.is_null() {
            return Err(_last_null_pointer_err("GDALGetRasterBand"));
        }

        Ok(VrtRasterBand {
            c_band,
            _dataset: self,
        })
    }
}

/// A raster band within a VRT dataset.
///
/// This struct provides methods specific to VRT bands, such as adding sources.
pub struct VrtRasterBand<'a> {
    c_band: GDALRasterBandH,
    _dataset: &'a VrtDataset,
}

impl<'a> VrtRasterBand<'a> {
    /// Returns the raw GDAL raster band handle.
    pub fn c_rasterband(&self) -> GDALRasterBandH {
        self.c_band
    }

    /// Adds a simple source to this VRT band.
    ///
    /// A simple source reads data from a rectangular region of a source band
    /// and writes it to a rectangular region of the VRT band.
    ///
    /// # Arguments
    /// * `source_band` - The source raster band to read from
    /// * `src_window` - Source window as `(x_offset, y_offset, x_size, y_size)` in pixels
    /// * `dst_window` - Destination window as `(x_offset, y_offset, x_size, y_size)` in pixels
    /// * `resampling` - Optional resampling method (e.g., "near", "bilinear", "cubic").
    ///   If None, uses default (nearest neighbor).
    /// * `nodata` - Optional nodata value for the source. If None, uses `NODATA_UNSET`.
    ///
    /// # Example
    /// ```rust, no_run
    /// use gdal::{Dataset};
    /// use gdal::vrt::VrtDataset;
    /// use gdal::raster::GdalDataType;
    ///
    /// # fn main() -> gdal::errors::Result<()> {
    /// let source = Dataset::open("source.tif")?;
    /// let source_band = source.rasterband(1)?;
    ///
    /// let mut vrt = VrtDataset::create(100, 100)?;
    /// vrt.add_band(GdalDataType::Float32, None)?;
    ///
    /// let vrt_band = vrt.rasterband(1)?;
    /// vrt_band.add_simple_source(
    ///     &source_band,
    ///     (0, 0, 100, 100),
    ///     (0, 0, 100, 100),
    ///     None,
    ///     None,
    /// )?;
    /// # Ok(())
    /// # }
    /// ```
    pub fn add_simple_source(
        &self,
        source_band: &RasterBand,
        src_window: (i32, i32, i32, i32),
        dst_window: (i32, i32, i32, i32),
        resampling: Option<&str>,
        nodata: Option<f64>,
    ) -> Result<()> {
        let c_resampling = resampling.and_then(|s| CString::new(s).ok());

        let resampling_ptr = c_resampling
            .as_ref()
            .map(|s| s.as_ptr())
            .unwrap_or(null_mut());

        let nodata_value = nodata.unwrap_or(NODATA_UNSET);

        let rv = unsafe {
            VRTAddSimpleSource(
                self.c_band as VRTSourcedRasterBandH,
                source_band.c_rasterband(),
                src_window.0, // nSrcXOff
                src_window.1, // nSrcYOff
                src_window.2, // nSrcXSize
                src_window.3, // nSrcYSize
                dst_window.0, // nDstXOff
                dst_window.1, // nDstYOff
                dst_window.2, // nDstXSize
                dst_window.3, // nDstYSize
                resampling_ptr,
                nodata_value,
            )
        };

        if rv != CPLErr::CE_None {
            return Err(_last_cpl_err(rv));
        }
        Ok(())
    }

    /// Sets the nodata value for this VRT band.
    ///
    /// # Arguments
    /// * `nodata` - The nodata value to set
    pub fn set_no_data_value(&self, nodata: f64) -> Result<()> {
        let rv = unsafe { gdal_sys::GDALSetRasterNoDataValue(self.c_band, nodata) };

        if rv != CPLErr::CE_None {
            return Err(_last_cpl_err(rv));
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_vrt_create() {
        let vrt = VrtDataset::create(100, 100).unwrap();
        assert_eq!(vrt.raster_count(), 0);
    }

    #[test]
    fn test_vrt_add_band() {
        let mut vrt = VrtDataset::create(100, 100).unwrap();
        let band_idx = vrt.add_band(GdalDataType::Float32, None).unwrap();
        assert_eq!(band_idx, 1);
        assert_eq!(vrt.raster_count(), 1);

        let band_idx = vrt.add_band(GdalDataType::UInt8, None).unwrap();
        assert_eq!(band_idx, 2);
        assert_eq!(vrt.raster_count(), 2);
    }

    #[test]
    fn test_vrt_set_geo_transform() {
        let mut vrt = VrtDataset::create(100, 100).unwrap();
        let transform = [0.0, 1.0, 0.0, 100.0, 0.0, -1.0];
        vrt.set_geo_transform(&transform).unwrap();
    }

    #[test]
    fn test_vrt_set_projection() {
        let mut vrt = VrtDataset::create(100, 100).unwrap();
        vrt.set_projection("EPSG:4326").unwrap();
    }

    #[test]
    fn test_nodata_unset_constant() {
        assert_eq!(NODATA_UNSET, -1234.56);
    }
}
