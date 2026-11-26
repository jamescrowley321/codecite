//! Print the SQLite version linked into the locked release rusqlite dependency.
fn main() {
    println!("{}", rusqlite::version());
}
