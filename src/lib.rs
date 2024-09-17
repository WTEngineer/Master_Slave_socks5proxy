#[no_mangle]
pub extern "C" fn my_function(param: i32) -> i32 {
    param * 2
}