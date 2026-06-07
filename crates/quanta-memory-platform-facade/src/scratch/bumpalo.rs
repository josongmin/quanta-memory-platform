#[derive(Debug, Default)]
pub struct BumpaloScratchArena {
    arena: bumpalo::Bump,
}

impl BumpaloScratchArena {
    pub fn arena(&self) -> &bumpalo::Bump {
        &self.arena
    }
}
