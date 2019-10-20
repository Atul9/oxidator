#version 450

layout(location = 0) in vec2 v_TexCoord;
layout(location = 0) out vec4 o_Target;

layout(set = 1, binding = 0) uniform texture2D t_pos;
layout(set = 1, binding = 1) uniform sampler s_pos;
layout(set = 0, binding = 3) uniform Locals {
    vec2 mouse_pos;
    vec2 resolution;
    vec2 inv_resolution;
    vec2 start_drag;
};

void main() {


    vec4 pos_attachment =  texture(sampler2D(t_pos, s_pos), v_TexCoord);
    vec3 mouse_pos_world = pos_attachment.xyz;

    vec3 color = vec3(1.0);

    //Cursor circle
    vec3 pos = texture(sampler2D(t_pos, s_pos), mouse_pos/resolution).xyz;
    float dist = length(mouse_pos_world - pos);
    float distance_to_sphere  = length(10.0 - dist);
    float base = clamp(1-distance_to_sphere,0.0,1.0);
    float alpha=  pow(base,2.0);

    // //Unit selection
    bool is_selected_area = pos_attachment.a >= 0.99;

    bool is_edge_area = false;
    for(int i = -1; i<= 1; i++){
        for(int j = -1; j<= 1; j++){
            if (!is_edge_area && (i!= 0 || j!= 0)){
                is_edge_area = is_edge_area || 
                texture(
                    sampler2D(t_pos, s_pos), 
                    v_TexCoord + vec2(i,j)/resolution).a >=0.99;
            }
        }
    }

    bool is_edge2_area = false;
    if (!is_edge_area){
    for(int i = -2; i<= 2; i++){
            for(int j = -2; j<= 2; j++){
                if ((max(i,j)==2 || min(i,j)==-2 ) && !is_edge2_area){
                    is_edge2_area = is_edge2_area||
                    texture(
                        sampler2D(t_pos, s_pos),
                        v_TexCoord + vec2(i,j)/resolution).a >=0.99;
                }
            }
        }
    }
    
    if (!is_selected_area && is_edge_area ){
        alpha = 1.0;
        color = vec3(0.0,1.0,0.0);
    } else if(!is_selected_area && is_edge2_area){
        alpha = 0.5;
        color = vec3(0.0,0.5,0.0);
    }

    if (start_drag != mouse_pos){
        uvec2 coord_screen = uvec2(v_TexCoord*resolution);
        vec2 min_ = vec2(min(start_drag.x,mouse_pos.x), min(start_drag.y,mouse_pos.y));
        vec2 max_ = vec2(max(start_drag.x,mouse_pos.x), max(start_drag.y,mouse_pos.y));

        if (coord_screen.x > min_.x && coord_screen.y > min_.y&&
        coord_screen.x < max_.x && coord_screen.y < max_.y){
            color = mix(vec3(0.0,1.0,0.3), color, 0.1);
            alpha = max(alpha, 0.1);
        } else if (coord_screen.x >= min_.x && coord_screen.y >= min_.y&&
        coord_screen.x <= max_.x && coord_screen.y <= max_.y){
            color = mix(vec3(0.0,0.7,0.1), color, 0.8);
            alpha = max(alpha, 0.8);
        }
    }

    o_Target = vec4(color,alpha);
}