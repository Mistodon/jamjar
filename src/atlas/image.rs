use std::borrow::Borrow;
use std::collections::HashMap;
use std::hash::Hash;

use image::RgbaImage;
use texture_packer::{TexturePacker, TexturePackerConfig};

use crate::{atlas::Atlas, draw::Region};

pub struct ImageAtlas<'a, K>
where
    K: ToOwned + Eq + Hash + ?Sized,
    K::Owned: Clone + Eq + Hash,
{
    regions: HashMap<K::Owned, Region>,
    source_images: HashMap<K::Owned, RgbaImage>,
    packer: TexturePacker<'a, RgbaImage>,
    pre_made_atlas: Option<RgbaImage>,
    backing_image_size: [u32; 2],
    available_area: ([u32; 2], [u32; 2]),
    modified: bool,
}

impl<'a, K> ImageAtlas<'a, K>
where
    K: ToOwned + Eq + Hash + ?Sized,
    K::Owned: Clone + Eq + Hash,
{
    fn config(size: [u32; 2]) -> TexturePackerConfig {
        TexturePackerConfig {
            max_width: size[0],
            max_height: size[1],
            allow_rotation: false,
            border_padding: 2,
            texture_padding: 2,
            trim: false,
            ..Default::default()
        }
    }

    pub fn new() -> Self {
        Self::with_size([4096, 4096])
    }

    pub fn with_size(backing_size: [u32; 2]) -> Self {
        Self::with_area_in_size(([0, 0], backing_size), backing_size)
    }

    pub fn with_area_in_size(
        (topleft, size): ([u32; 2], [u32; 2]),
        backing_size: [u32; 2],
    ) -> Self {
        ImageAtlas {
            regions: Default::default(),
            source_images: Default::default(),
            packer: TexturePacker::new_skyline(Self::config(size)),
            pre_made_atlas: None,
            backing_image_size: backing_size,
            available_area: (topleft, size),
            modified: true,
        }
    }

    pub fn pre_made(
        atlas_image: RgbaImage,
        regions: HashMap<K::Owned, Region>,
        backing_size: [u32; 2],
    ) -> Self {
        let [bw, bh] = backing_size;

        let mut packer = TexturePacker::new_skyline(Self::config([bw, bh]));
        packer.pack_own(String::new(), atlas_image.clone()).unwrap();
        let frame = packer.get_frame("").unwrap().frame;

        assert!(
            frame.x == 0 && frame.y == 1,
            "Oops, that's not how I thought this worked"
        );

        ImageAtlas {
            regions,
            source_images: Default::default(),
            packer,
            pre_made_atlas: Some(atlas_image),
            backing_image_size: [bw, bh],
            available_area: ([0, 0], [bw, bh]),
            modified: true,
        }
    }

    pub fn compile(&mut self) -> RgbaImage {
        let [bw, bh] = self.backing_image_size;
        let mut atlas = RgbaImage::new(bw, bh);
        self.compile_into(&mut atlas);
        atlas
    }
}

impl<'a, K> Atlas<(K::Owned, RgbaImage), K, Region, RgbaImage> for ImageAtlas<'a, K>
where
    K: ToOwned + Eq + Hash + ?Sized,
    K::Owned: Clone + Eq + Hash,
{
    fn insert(&mut self, (key, image): (K::Owned, RgbaImage)) {
        let string_key = self.source_images.len().to_string();
        self.packer
            .pack_own(string_key.clone(), image.clone())
            .unwrap();
        let texture_packer::Rect { x, y, w, h } = self.packer.get_frame(&string_key).unwrap().frame;

        let [bw, bh] = self.backing_image_size;
        let [bw, bh] = [bw as f32, bh as f32];
        let ([ax, ay], _) = self.available_area;

        let region = Region {
            pixels: ([x + ax, y + ay], [w, h]),
            uv: (
                [(ax + x) as f32 / bw, (ay + y) as f32 / bh],
                [w as f32 / bw, h as f32 / bh],
            ),
        };

        self.regions.insert(key.clone(), region);
        self.source_images.insert(key, image);
        self.modified = true;
    }

    fn fetch(&self, key: &K) -> Region {
        self.regions[key]
    }

    fn compile_into(&mut self, dest: &mut RgbaImage) -> bool {
        use image::GenericImage;

        let ([ax, ay], _) = self.available_area;
        if let Some(pre_made_atlas) = &self.pre_made_atlas {
            dest.copy_from(pre_made_atlas, ax, ay).unwrap();
        }

        for (key, region) in self.regions.iter() {
            let image = self.source_images.get(key.borrow());

            // If there's no image, this region must be from the pre-made atlas
            if let Some(image) = image {
                dest.copy_from(image, region.pixels.0[0], region.pixels.0[1])
                    .unwrap();
            }
        }

        self.modified = false;

        true
    }

    fn modified(&self) -> bool {
        self.modified
    }
}
