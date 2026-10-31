// Lumetri's full precision GPU passes. The table layout is owned by render::lumetri_gpu.
@group(0) @binding(4) var<storage, read> lt: array<f32>;
fn lv(i:u32)->vec3<f32> {return vec3(lt[i],lt[i+1u],lt[i+2u]);}
fn pq_enc(x:f32)->f32 {
    let y=powf(clamp(x,0.0,1.0),2610.0/16384.0);
    return powf((3424.0/4096.0+2413.0/128.0*y)/(1.0+2392.0/128.0*y),2523.0/32.0);
}
fn pq_dec(x:f32)->f32 {
    let p=powf(clamp(x,0.0,1.0),32.0/2523.0);
    return powf(max(p-3424.0/4096.0,0.0)/max(2413.0/128.0-2392.0/128.0*p,1e-20),16384.0/2610.0);
}
fn hlg_enc(x:f32)->f32 {
    if x<=1.0/12.0 {return sqrt(max(3.0*x,0.0));}
    return 0.17883277*log(12.0*x-0.28466892)+0.5599107;
}
fn hlg_dec(x:f32)->f32 {
    if x<=0.5 {return x*x/3.0;}
    return (exp((x-0.5599107)/0.17883277)+0.28466892)/12.0;
}
fn grade_enc(c:vec3<f32>,base:u32)->vec3<f32> {
    if lt[base]==0.0 {return enc(c);}
    if lt[base]==1.0 {
        let v=max(c,vec3(0.0))*203.0/10000.0;
        return vec3(pq_enc(v.x),pq_enc(v.y),pq_enc(v.z))/lt[base+2u];
    }
    let v=max(c,vec3(0.0))*203.0/lt[base+1u];
    let yd=dot(v,vec3(0.2627,0.6780,0.0593));
    if yd<=0.0 {return vec3(0.0);}
    let es=v*powf(yd,(1.0-lt[base+3u])/lt[base+3u]);
    return vec3(hlg_enc(es.x),hlg_enc(es.y),hlg_enc(es.z));
}
fn grade_dec(c:vec3<f32>,base:u32)->vec3<f32> {
    if lt[base]==0.0 {return dec(c);}
    if lt[base]==1.0 {
        let v=c*lt[base+2u];
        return vec3(pq_dec(v.x),pq_dec(v.y),pq_dec(v.z))*10000.0/203.0;
    }
    let q=clamp(c,vec3(0.0),vec3(1.5));
    let es=vec3(hlg_dec(q.x),hlg_dec(q.y),hlg_dec(q.z));
    let ys=dot(es,vec3(0.2627,0.6780,0.0593));
    if ys<=0.0 {return vec3(0.0);}
    return es*powf(ys,lt[base+3u]-1.0)*lt[base+1u]/203.0;
}
fn unit_scale(v:vec3<f32>,base:u32)->f32 {
    if lt[base]!=0.0 {return max(max(max(v.x,v.y),v.z),1.0);}
    return 1.0;
}
fn grade_unit(v:vec3<f32>,base:u32)->vec3<f32> {
    if lt[base]!=0.0 {return max(v/unit_scale(v,base),vec3(0.0));}
    return clamp(v,vec3(0.0),vec3(1.0));
}
fn table_rgb(offset:u32,index:u32)->vec3<f32> {return lv(offset+index*3u);}
fn cube_at(offset:u32,n:u32,p:vec3<u32>)->vec3<f32> {return table_rgb(offset,p.x+p.y*n+p.z*n*n);}
fn apply_lut(c:vec3<f32>,descriptor:u32)->vec3<f32> {
    if descriptor==0u {return c;}
    var v=c;
    let shaper=u32(lt[descriptor]); let sn=u32(lt[descriptor+1u]);
    if sn>=2u {
        let coord=clamp((v-lv(descriptor+2u))/(lv(descriptor+5u)-lv(descriptor+2u)),vec3(0.0),vec3(1.0))*f32(sn-1u);
        for(var k=0u;k<3u;k++) {
            let i=min(u32(coord[k]),sn-2u);let f=coord[k]-f32(i);
            v[k]=mix(table_rgb(shaper,i)[k],table_rgb(shaper,i+1u)[k],f);
        }
    }
    let offset=u32(lt[descriptor+8u]); let n=u32(lt[descriptor+9u]);
    if n<2u {return v;}
    let coord=clamp((v-lv(descriptor+10u))/(lv(descriptor+13u)-lv(descriptor+10u)),vec3(0.0),vec3(1.0))*f32(n-1u);
    let p=min(vec3<u32>(coord),vec3(n-2u)); let f=coord-vec3<f32>(p);
    var a=vec3(0u); var b=vec3(0u); var weights=vec4(0.0);
    if f.x>f.y {
        if f.y>f.z {a=vec3(1u,0u,0u);b=vec3(1u,1u,0u);weights=vec4(1.0-f.x,f.x-f.y,f.y-f.z,f.z);}
        else if f.x>f.z {a=vec3(1u,0u,0u);b=vec3(1u,0u,1u);weights=vec4(1.0-f.x,f.x-f.z,f.z-f.y,f.y);}
        else {a=vec3(0u,0u,1u);b=vec3(1u,0u,1u);weights=vec4(1.0-f.z,f.z-f.x,f.x-f.y,f.y);}
    } else if f.z>f.y {a=vec3(0u,0u,1u);b=vec3(0u,1u,1u);weights=vec4(1.0-f.z,f.z-f.y,f.y-f.x,f.x);}
    else if f.z>f.x {a=vec3(0u,1u,0u);b=vec3(0u,1u,1u);weights=vec4(1.0-f.y,f.y-f.z,f.z-f.x,f.x);}
    else {a=vec3(0u,1u,0u);b=vec3(1u,1u,0u);weights=vec4(1.0-f.y,f.y-f.x,f.x-f.z,f.z);}
    return cube_at(offset,n,p)*weights.x+cube_at(offset,n,p+a)*weights.y+cube_at(offset,n,p+b)*weights.z+cube_at(offset,n,p+vec3(1u))*weights.w;
}
fn lut_unit(v:vec3<f32>,base:u32,descriptor:u32)->vec3<f32> {
    return apply_lut(grade_unit(v,base),descriptor)*unit_scale(v,base);
}
fn curve_sample(slot:u32,x:f32)->f32 {
    let offset=u32(lt[96u+slot]);
    if offset==0u {return x;}
    if lt[4]!=0.0 && x>1.0 {return lt[offset+1023u]+x-1.0;}
    let pos=clamp(x,0.0,1.0)*1023.0;let i=u32(pos);
    return mix(lt[offset+i],lt[offset+min(i+1u,1023u)],pos-f32(i));
}
fn s_curve(x:f32,k:f32)->f32 {
    let q=clamp(x,0.0,1.0);return q+(q*q*(3.0-2.0*q)-q)*k;
}
fn look_curve(v:vec3<f32>,k:f32)->vec3<f32> {return vec3(s_curve(v.x,k),s_curve(v.y,k),s_curve(v.z,k));}
fn procedural_look(v:vec3<f32>,look:u32)->vec3<f32> {
    let l=luma709(v);
    switch look {
        case 1u:{return look_curve(v+vec3(0.0,0.08,0.1)*(1.0-l)+vec3(0.1,0.04,-0.06)*l,0.35);}
        case 2u:{return mix(v*vec3(1.06,1.0,0.9)+vec3(0.02,0.01,0.0),vec3(l),0.15)*0.94+vec3(0.04);}
        case 3u:{return look_curve(v*vec3(0.9,0.98,1.1)+vec3(0.0,0.0,0.02),0.2);}
        case 4u:{return look_curve(mix(v,vec3(l),0.55),0.6);}
        case 5u:{return vec3(0.08)+mix(v,vec3(l),0.25)*0.84;}
        case 6u:{return look_curve(vec3(l),0.3);}
        case 7u:{return look_curve(v*vec3(1.1,1.02,0.82)+vec3(0.03,0.01,0.0),0.15);}
        case 8u:{return v*vec3(0.75,0.85,1.15)*0.8;}
        default:{return v;}
    }
}
fn basic_full(c:vec3<f32>,p:vec2<i32>)->vec3<f32> {
    var input=c;
    if lt[108]!=0.0 {input=grade_dec(lut_unit(grade_enc(input,0u),0u,u32(lt[108])),0u);}
    var v=grade_enc(input*lv(8u),0u);
    v=(v-vec3(lt[12]))/max(lt[13]-lt[12],1e-3);
    let l=luma709(v); let sh=clamp(1.0-l,0.0,1.0);let hi=clamp(l,0.0,1.0);
    let nl=max(l+lt[14]*sh*sh*sh+lt[15]*hi*hi*hi,0.0);
    if l>1e-5 {v*=nl/l;}
    if abs(lt[16])>1e-4 {
        for(var k=0u;k<3u;k++) {
            if lt[0]!=0.0 && v[k]>1.0 {continue;}
            let q=clamp(v[k],0.0,1.0);let gain=1.0+lt[16];
            if gain>=1.0 {v[k]=q+(q*q*(3.0-2.0*q)-q)*(gain-1.0);} else {v[k]=0.5+(q-0.5)*gain;}
        }
    }
    if lt[0]!=0.0 && abs(lt[11])>1e-4 {v=select(v,vec3(1.0)+(v-vec3(1.0))*max(1.0+lt[11],0.0),v>vec3(1.0));}
    if lt[17]>0.0 {v=v*(1.0-0.25*lt[17])+vec3(0.12*lt[17]);}
    if lt[23]!=0.0 {let l2=clamp(luma709(v),0.0,1.0);v+=(lv(20u)-vec3(0.5))*0.3*(1.0-l2)+(lv(24u)-vec3(0.5))*0.3*l2;}
    let l3=luma709(v);let sat=max(max(v.x,v.y),v.z)-min(min(v.x,v.y),v.z);
    v=vec3(l3)+(v-vec3(l3))*lt[18]*(1.0+lt[19]*(1.0-clamp(sat,0.0,1.0)));
    if abs(lt[27])>1e-4 {
        let q=(vec2<f32>(p)/vec2<f32>(size())-vec2(0.5))*2.0;
        let distance=length(q*vec2(lt[29],1.0))/sqrt(2.0);
        let edge=clamp((distance-lt[28]*0.9)/(max(lt[30],0.01)*0.9),0.0,1.0);
        let gain=1.0+lt[27]*0.2*edge*edge*(3.0-2.0*edge);
        if lt[27]<0.0 {v*=max(gain,0.0);} else {v+=(vec3(1.0)-v)*(gain-1.0);}
    }
    return grade_dec(v,0u);
}
fn advanced_full(c:vec3<f32>)->vec3<f32> {
    var enabled_stage=lt[32]>0.0 || lt[109]!=0.0;
    let wheel_active=lt[34]!=0.0 && (any(abs(lv(36u))>vec3(1e-5))||any(abs(lv(40u))>vec3(1e-5))||any(abs(lv(44u))>vec3(1e-5))||abs(lt[39])+abs(lt[43])+abs(lt[47])>1e-5);
    for(var i=96u;i<105u;i++) {enabled_stage=enabled_stage || lt[i]!=0.0;}
    if !enabled_stage && !wheel_active {return c;}
    var v=grade_enc(c,4u);
    if lt[109]!=0.0 {v=mix(v,lut_unit(v,4u,u32(lt[109])),lt[33]);}
    else if lt[32]>0.0 {
        var look=v;
        if lt[4]!=0.0 {look=procedural_look(grade_unit(v,4u),u32(lt[32]))*unit_scale(v,4u);}
        else {look=procedural_look(v,u32(lt[32]));}
        v=mix(v,look,lt[33]);
    }
    if wheel_active {
        let l=clamp(luma709(v),0.0,1.0);let sh=(1.0-l)*(1.0-l);let hi=l*l;let mid=max(1.0-sh-hi,0.0);
        v+=(lv(36u)+vec3(lt[39]))*0.3*sh;
        v+=(lv(40u)+vec3(lt[43]))*0.3*mid;
        v*=vec3(1.0)+(lv(44u)+vec3(lt[47]))*0.5*hi;
    }
    for(var k=0u;k<3u;k++) {v[k]=curve_sample(0u,v[k]);v[k]=curve_sample(k+1u,v[k]);}
    var hue_active=false;for(var i=100u;i<105u;i++) {hue_active=hue_active || lt[i]!=0.0;}
    if hue_active {
        let scale=unit_scale(v,4u);var h=rgb_to_hsl(grade_unit(v,4u));let original=h;
        if lt[101]!=0.0 {h.x=fract_euclid(h.x+curve_sample(5u,original.x)-0.5);}
        var sat=1.0;
        if lt[100]!=0.0 {sat*=curve_sample(4u,original.x)*2.0;}
        if lt[103]!=0.0 {sat*=curve_sample(7u,original.z)*2.0;}
        if lt[104]!=0.0 {sat*=curve_sample(8u,original.y)*2.0;}
        h.y=clamp(h.y*sat,0.0,1.0);
        if lt[102]!=0.0 {h.z=clamp(h.z+(curve_sample(6u,original.x)-0.5)*0.5,0.0,1.0);}
        v=hsl_to_rgb(h.x,h.y,h.z)*scale;
    }
    return grade_dec(v,4u);
}
fn hsl_key_full(c:vec3<f32>)->f32 {
    let h=rgb_to_hsl(grade_unit(grade_enc(c,4u),4u));let d=abs(h.x-lt[48]);
    let mh=1.0-clamp((min(d,1.0-d)-lt[49]/2.0)/lt[53],0.0,1.0);
    let ms=clamp((h.y-lt[50])/lt[53],0.0,1.0);
    let ml=min(clamp((h.z-lt[51])/lt[53],0.0,1.0),clamp((lt[52]-h.z)/lt[53],0.0,1.0));
    return mh*ms*ml;
}
fn hsl_correct(c:vec3<f32>,mask:f32)->vec3<f32> {
    let v=grade_enc(c,4u);let q=grade_unit(v,4u);let h=rgb_to_hsl(q);var result=q;
    if lt[54]==1.0 {result=mix(vec3(h.z),q,mask);}
    else if lt[54]==2.0 {result=q*mask;}
    else if lt[54]==3.0 {result=vec3(mask);}
    else {
        let corrected=hsl_to_rgb(fract_euclid(h.x+lt[58]),clamp(h.y*lt[57],0.0,1.0),h.z)*vec3(1.0+0.25*lt[55],1.0-0.2*lt[56],1.0-0.25*lt[55]);
        result=mix(q,corrected,mask)*unit_scale(v,4u);
    }
    return grade_dec(result,4u);
}
fn selective_full(c:vec3<f32>)->vec3<f32> {
    let v=grade_enc(c,4u);let original=rgb_to_hsl(grade_unit(v,4u));var delta=vec3(0.0);
    for(var i=0u;i<6u;i++) {
        let base=64u+5u*i;let d=abs(original.x-lt[base]);let x=clamp(1.0-min(d,1.0-d)/lt[base+1u],0.0,1.0);
        let weight=x*x*(3.0-2.0*x)*clamp(original.y/0.05,0.0,1.0);
        delta+=lv(base+2u)*weight;
    }
    let h=vec3(fract_euclid(original.x+delta.x),clamp(original.y*(1.0+delta.y),0.0,1.0),clamp(original.z+delta.z*0.5,0.0,1.0));
    return grade_dec(hsl_to_rgb(h.x,h.y,h.z)*unit_scale(v,4u),4u);
}
fn lumetri_full(op:u32,p:vec2<i32>)->vec4<f32> {
    let o=ld(p);
    if op==OP_LUMETRI_HSL_KEY {var m=0.0;if o.a>1e-6 {m=hsl_key_full(o.rgb/o.a);}return vec4(m);}
    if op==OP_LUMETRI_HSL_CORRECT {
        let original=textureLoad(aux,p,0);if original.a<=1e-6 {return original;}
        return vec4(hsl_correct(original.rgb/original.a,o.x)*original.a,original.a);
    }
    if o.a<=1e-6 {return o;}
    let c=o.rgb/o.a;var result=c;
    if op==OP_LUMETRI_BASIC {result=basic_full(c,p);}
    else if op==OP_LUMETRI_ADVANCED {result=advanced_full(c);}
    else if op==OP_LUMETRI_SELECTIVE {result=selective_full(c);}
    return vec4(result*o.a,o.a);
}
// Display transform after compositing in the sequence's working primaries.
fn output_eetf(nits:f32,source:f32,dst_peak:f32)->f32 {
    if dst_peak>=source || nits<=0.0 {return clamp(nits,0.0,max(source,dst_peak));}
    let smax=pq_enc(source/10000.0);let e1=min(pq_enc(nits/10000.0)/smax,1.0);
    let max_lum=pq_enc(dst_peak/10000.0)/smax;let ks=1.5*max_lum-0.5;var e2=e1;
    if e1>=ks {
        let t=(e1-ks)/(1.0-ks);let t2=t*t;let t3=t2*t;
        e2=(2.0*t3-3.0*t2+1.0)*ks+(t3-2.0*t2+t)*(1.0-ks)+(-2.0*t3+3.0*t2)*max_lum;
    }
    return pq_dec(e2*smax)*10000.0;
}
fn output_linear(input:vec3<f32>)->vec3<f32> {
    var c=input;
    if u.i1.x!=0u {
        let m=max(max(c.x,c.y),c.z);
        if m>0.0 {
            let nits=m*203.0;var gain=1.0;
            if nits>=u.p2.y {gain=u.p2.z/203.0/m;}
            else if nits>1e-6 {gain=output_eetf(nits,u.p2.y,u.p2.z)/nits;}
            c*=gain;
        }
    }
    c=vec3(dot(u.p0.xyz,c),dot(vec3(u.p0.w,u.p1.xy),c),dot(vec3(u.p1.zw,u.p2.x),c));
    if u.i1.y!=0u {
        let mn=min(min(c.x,c.y),c.z);
        if mn<0.0 {
            let y=dot(vec3(u.p2.w,u.p3.xy),c);
            if y<=0.0 {return vec3(0.0);}
            c=max(vec3(y)+(c-vec3(y))*y/(y-mn),vec3(0.0));
        }
    }
    return c;
}

// Keep median's private scratch array out of the ordinary colour kernel's call graph.
// Otherwise Metal reserves its per-thread scratch for every colour pixel, hurting occupancy.
@compute @workgroup_size(8,8,1)
fn lumetri_median(@builtin(global_invocation_id) gid:vec3<u32>) {
    let p=vec2<i32>(gid.xy);
    if any(p>=size()) {return;}
    let o=ld(p);

        var values:array<f32,289>;var count=0u;let r=i32(u.i1.x);
        for(var y=max(0,p.y-r);y<min(size().y,p.y+r+1);y++) {
            for(var x=max(0,p.x-r);x<min(size().x,p.x+r+1);x++) {
                let v=ld(vec2(x,y)).x;var j=count;
                loop {if j==0u {break;} if values[j-1u]<=v {break;} values[j]=values[j-1u];j--;}
                values[j]=v;count++;
            }
        }
        textureStore(dst,p,vec4(mix(o.x,values[count/2u],min(lt[59],1.0))));

}
