//! Incrusta el icono de Whatfast en el ejecutable de Windows.
//!
//! Sin esto, el Explorador, la barra de tareas y el alt-tab muestran el
//! icono generico de Windows. En los otros sistemas el recurso no existe:
//! la app usa su propio icono en la ventana y el icono del archivo lo pone
//! el instalador de cada plataforma.

fn main() {
    println!("cargo:rerun-if-changed=assets/icon.rc");
    println!("cargo:rerun-if-changed=assets/icon.ico");
    // El recurso es del ejecutable de Windows, no de la maquina que compila:
    // `CARGO_CFG_TARGET_OS` es el sistema de destino, asi que el icono se
    // incrusta tambien cuando el destino es Windows y la compilacion corre en
    // Linux o en macOS. Con `#[cfg(windows)]` el recurso se incrustaba en el
    // sistema del host, que es el equivocado en una compilacion cruzada.
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("windows") {
        return;
    }
    match embed_resource::compile("assets/icon.rc", embed_resource::NONE).manifest_optional() {
        Ok(()) => {}
        Err(error) => {
            // El icono es un extra: si el compilador de recursos no esta en
            // la maquina, la compilacion sigue igual.
            println!("cargo:warning=el icono de Whatfast no se incrusto: {error:?}");
        }
    }
}
