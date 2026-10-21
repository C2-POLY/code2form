module lib_import() import("example009.dxf", layer="body");
function lib_dim() = dxf_dim(file="example009.dxf", name="bodywidth");
