//! Rewrites the committed MCP tool reference from the live registry.

fn main() -> anyhow::Result<()> {
    let reference = trogon_atlas_mcp::reference::generate()?;
    let path = trogon_atlas_mcp::reference::reference_file();
    std::fs::write(&path, reference)?;
    Ok(())
}
