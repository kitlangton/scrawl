// Frozen approved movie identity. Preparation verifies every byte before removal.
export const movie = {
  path: "/Apparent-ghost-demo-v4-lilac.mp4",
  size: 30999148,
  sha256: "89c4621457e573d58a00f99714682eb4bf9351b59de95cabebb944007654d7a7",
  chunkSize: 8388608,
  chunks: [
    { size: 8388608, sha256: "ec05807d6bba777b27b51faa78264b10c1f9371d69ac1a952cf539f527f8f40d" },
    { size: 8388608, sha256: "7cb61028caf74affcc5137d6b56b3481ba3ae9ae6936acbb6e778ffe22af8eb3" },
    { size: 8388608, sha256: "2117b1bf243710097ec4f9be61772da481ed4b7468d011705a79a4f8734d0dc9" },
    { size: 5833324, sha256: "8d145493d824be9dc4b454b6f3f64b5ed1efb3ee930c3e1c24823d1a30dc479a" },
  ],
}
export const chunkPath = (chunk) => `/assets/apparent-video-${chunk.sha256}.bin`
export const movieETag = `"sha256-${movie.sha256}"`
