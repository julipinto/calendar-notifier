use serde::Deserialize;

#[derive(Deserialize)]
struct ClientFile {
    installed: ClientCreds,
}

/// Credenciais OAuth do Google (tipo "Desktop app").
#[derive(Deserialize, Clone)]
pub struct ClientCreds {
    pub client_id: String,
    pub client_secret: String,
    pub auth_uri: String,
    pub token_uri: String,
}

/// Lê as credenciais embutidas em tempo de compilação a partir de
/// `src-tauri/google_client.json`. Esse arquivo é gitignored (fora do
/// versionamento), mas o compilador o inclui aqui. Num app desktop o
/// client_secret não é sigiloso — ele é distribuído no binário de qualquer forma.
pub fn client_creds() -> ClientCreds {
    const RAW: &str = include_str!("../google_client.json");
    let parsed: ClientFile =
        serde_json::from_str(RAW).expect("google_client.json inválido ou ausente");
    parsed.installed
}
