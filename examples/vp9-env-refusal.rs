use ps_media_codecs::{CodecError, Vp9Encoder};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    if !std::env::vars_os().any(|(name, _)| name.to_string_lossy().starts_with("VP9_")) {
        return Err(std::io::Error::other(
            "run with a synthetic VP9_* override to exercise refusal",
        )
        .into());
    }
    match Vp9Encoder::new() {
        Err(CodecError::InvalidInput(_)) => {
            println!("VP9 constructor refused process overrides before upstream initialization");
            Ok(())
        }
        Err(error) => Err(error.into()),
        Ok(_) => Err(std::io::Error::other("encoder accepted a process override").into()),
    }
}
