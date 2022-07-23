use crate::camera::Camera;
use crate::vertex::{self, QuadListRenderData, Vertex};
use glam::Vec3;

pub struct OrthoProjCoords {
    pub left: f32,
    pub right: f32,
    pub bottom: f32,
    pub top: f32,
    pub near: f32,
    pub far: f32,
}

pub struct LightUniform {
    pub position: glam::Vec3,
    pub color: glam::Vec3,
    pub sun_position: glam::Vec3,
    pub sun_position_camera_adjusted: glam::Vec3,
    pub sun_target: glam::Vec3,
    pub sun_target_camera_adjusted: glam::Vec3,
    pub sunlight_ortho_proj_coords: OrthoProjCoords,
    pub sunlight_ortho_proj: glam::Mat4,
}

#[repr(C)]
#[derive(Debug, Copy, Clone, bytemuck::Pod, bytemuck::Zeroable)]
pub struct LightUniformRaw {
    position: [f32; 3],
    _padding: u32,
    color: [f32; 3],
    _padding2: u32,
    light_space_matrix: [[f32; 4]; 4],
}

impl LightUniform {
    pub fn new(
        position: glam::Vec3,
        color: glam::Vec3,
        sun_position: glam::Vec3,
        sunlight_ortho_proj_coords: OrthoProjCoords,
    ) -> Self {
        let sunlight_ortho_proj = glam::Mat4::orthographic_rh(
            sunlight_ortho_proj_coords.left,
            sunlight_ortho_proj_coords.right,
            sunlight_ortho_proj_coords.bottom,
            sunlight_ortho_proj_coords.top,
            sunlight_ortho_proj_coords.near,
            sunlight_ortho_proj_coords.far,
        );
        Self {
            position,
            color,
            sun_position,
            sun_position_camera_adjusted: sun_position,
            sun_target: [0.0, 0.0, 0.0].into(),
            sun_target_camera_adjusted: [0.0, 0.0, 0.0].into(),
            sunlight_ortho_proj_coords,
            sunlight_ortho_proj,
        }
    }

    pub fn to_raw(&self) -> LightUniformRaw {
        let light_view = glam::Mat4::look_at_rh(
            self.sun_position_camera_adjusted.into(),
            self.sun_target_camera_adjusted.into(),
            [0.0, 1.0, 0.0].into(),
        );

        let light_space_matrix = (self.sunlight_ortho_proj * light_view).to_cols_array_2d();

        LightUniformRaw {
            position: self.position.into(),
            _padding: 0,
            color: self.color.into(),
            _padding2: 0,
            light_space_matrix,
        }
    }

    pub fn update_light_space_proj(&mut self, camera: &Camera) {
        let sun_y_adjust = camera.initial_eye.y - camera.eye.y;

        self.sun_position_camera_adjusted = self.sun_position;
        self.sun_target_camera_adjusted = self.sun_target;
        self.sun_position_camera_adjusted.y += sun_y_adjust;
        self.sun_target_camera_adjusted.y += sun_y_adjust;
    }

    pub fn vertex_data_for_sunlight_proj(&self) -> QuadListRenderData {
        let oc = &self.sunlight_ortho_proj_coords;

        let light_view = glam::Mat4::look_at_rh(
            self.sun_position_camera_adjusted.into(),
            self.sun_target_camera_adjusted.into(),
            [0.0, 1.0, 0.0].into(),
        );

        vertex::Vertex::generate_quad_data(
            &vec![
                // left face
                [
                    Vec3::new(oc.left, oc.top, oc.far),
                    Vec3::new(oc.left, oc.top, oc.near),
                    Vec3::new(oc.left, oc.bottom, oc.near),
                    Vec3::new(oc.left, oc.bottom, oc.far),
                ],
                // right face
                [
                    Vec3::new(oc.right, oc.top, oc.near),
                    Vec3::new(oc.right, oc.top, oc.far),
                    Vec3::new(oc.right, oc.bottom, oc.far),
                    Vec3::new(oc.right, oc.bottom, oc.near),
                ],
                // bottom face
                [
                    Vec3::new(oc.left, oc.bottom, oc.far),
                    Vec3::new(oc.left, oc.bottom, oc.near),
                    Vec3::new(oc.right, oc.bottom, oc.near),
                    Vec3::new(oc.right, oc.bottom, oc.far),
                ],
                // top face
                [
                    Vec3::new(oc.right, oc.top, oc.far),
                    Vec3::new(oc.right, oc.top, oc.near),
                    Vec3::new(oc.left, oc.top, oc.near),
                    Vec3::new(oc.left, oc.top, oc.far),
                ],
                // near face
                [
                    Vec3::new(oc.left, oc.top, oc.near),
                    Vec3::new(oc.right, oc.top, oc.near),
                    Vec3::new(oc.right, oc.bottom, oc.near),
                    Vec3::new(oc.left, oc.bottom, oc.near),
                ],
                // far face
                [
                    Vec3::new(oc.left, oc.top, oc.far),
                    Vec3::new(oc.right, oc.top, oc.far),
                    Vec3::new(oc.right, oc.bottom, oc.far),
                    Vec3::new(oc.left, oc.bottom, oc.far),
                ],
            ],
            Some(light_view),
        )
    }
}
