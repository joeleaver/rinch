fn main() {
    rinch::App::new(paint::app)
        .title("Paint Demo")
        .size(1150, 750)
        .run();
}
