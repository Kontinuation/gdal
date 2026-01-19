//! Tests for VRT C API bindings

use gdal::vrt::NODATA_UNSET;
use gdal_sys::{
    GDALAllRegister, GDALClose, GDALDataType, GDALGetRasterBand, GDALGetRasterCount,
    GDALSetGeoTransform, GDALSetRasterNoDataValue, VRTAddBand, VRTCreate, VRTDatasetH,
    VRTFlushCache,
};
use std::ptr::null_mut;

/// Test creating a VRT dataset using the low-level C API
#[test]
fn test_vrt_create_basic() {
    unsafe {
        // Register all GDAL drivers first
        GDALAllRegister();

        // Create a 100x100 VRT dataset
        let vrt_ds: VRTDatasetH = VRTCreate(100, 100);
        assert!(!vrt_ds.is_null(), "VRTCreate should not return null");

        // Check initial band count
        let initial_count = GDALGetRasterCount(vrt_ds as _);
        println!("Initial band count: {}", initial_count);

        // Add a band - VRTAddBand returns the band count after adding, not the band number
        let result = VRTAddBand(vrt_ds, GDALDataType::GDT_Float32, null_mut());
        println!("VRTAddBand result: {}", result);

        // Check band count after adding
        let band_count = GDALGetRasterCount(vrt_ds as _);
        println!("Band count after VRTAddBand: {}", band_count);
        assert_eq!(band_count, 1, "Should have 1 band after VRTAddBand");

        // Get the band handle
        let band_handle = GDALGetRasterBand(vrt_ds as _, 1);
        assert!(!band_handle.is_null(), "Band handle should not be null");

        // Set nodata value
        let nodata = -9999.0;
        GDALSetRasterNoDataValue(band_handle, nodata);

        // Set geotransform
        let mut gt = [0.0, 1.0, 0.0, 100.0, 0.0, -1.0];
        GDALSetGeoTransform(vrt_ds as _, gt.as_mut_ptr());

        // Flush cache
        VRTFlushCache(vrt_ds);

        // Close the dataset
        GDALClose(vrt_ds as _);
    }
}

/// Test that NODATA_UNSET constant has the expected value
#[test]
fn test_vrt_nodata_unset() {
    assert_eq!(NODATA_UNSET, -1234.56);
}
