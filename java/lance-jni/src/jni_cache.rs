// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: Copyright The Lance Authors

//! Process-wide cache of JNI class refs and method IDs.
//!
//! Initialized once from `JNI_OnLoad` so application classes resolve under the
//! correct classloader. Once populated, hot-path callers use `_unchecked` JNI
//! variants and skip per-call `find_class` + JVM type-signature parsing
//! (which previously dominated `getZonemapStats` CPU per async-profiler).

use crate::Result;
use jni::JNIEnv;
use jni::objects::{GlobalRef, JMethodID, JStaticMethodID};
use std::sync::OnceLock;

pub static JNI_CACHE: OnceLock<JniCache> = OnceLock::new();

pub struct JniCache {
    pub long_class: GlobalRef,
    pub long_ctor: JMethodID, // <init>(J)V

    pub double_class: GlobalRef,
    pub double_ctor: JMethodID, // <init>(D)V

    pub boolean_class: GlobalRef,
    pub boolean_ctor: JMethodID, // <init>(Z)V

    pub float_class: GlobalRef,
    pub float_ctor: JMethodID, // <init>(F)V

    pub array_list_class: GlobalRef,
    pub array_list_ctor: JMethodID, // <init>()V
    pub array_list_add: JMethodID,  // add(Ljava/lang/Object;)Z

    pub hash_map_class: GlobalRef,
    pub hash_map_ctor: JMethodID, // <init>()V
    pub hash_map_put: JMethodID,  // put(LObject;LObject;)LObject;

    pub optional_class: GlobalRef,
    pub optional_of_nullable: JStaticMethodID, // ofNullable(LObject;)LOptional;
}

impl JniCache {
    pub fn init(env: &mut JNIEnv) -> Result<Self> {
        fn resolve_class(env: &mut JNIEnv, name: &str) -> Result<GlobalRef> {
            let local = env.find_class(name)?;
            Ok(env.new_global_ref(local)?)
        }

        let long_class = resolve_class(env, "java/lang/Long")?;
        let long_ctor = env.get_method_id(&long_class, "<init>", "(J)V")?;

        let double_class = resolve_class(env, "java/lang/Double")?;
        let double_ctor = env.get_method_id(&double_class, "<init>", "(D)V")?;

        let boolean_class = resolve_class(env, "java/lang/Boolean")?;
        let boolean_ctor = env.get_method_id(&boolean_class, "<init>", "(Z)V")?;

        let float_class = resolve_class(env, "java/lang/Float")?;
        let float_ctor = env.get_method_id(&float_class, "<init>", "(F)V")?;

        let array_list_class = resolve_class(env, "java/util/ArrayList")?;
        let array_list_ctor = env.get_method_id(&array_list_class, "<init>", "()V")?;
        let array_list_add =
            env.get_method_id(&array_list_class, "add", "(Ljava/lang/Object;)Z")?;

        let hash_map_class = resolve_class(env, "java/util/HashMap")?;
        let hash_map_ctor = env.get_method_id(&hash_map_class, "<init>", "()V")?;
        let hash_map_put = env.get_method_id(
            &hash_map_class,
            "put",
            "(Ljava/lang/Object;Ljava/lang/Object;)Ljava/lang/Object;",
        )?;

        let optional_class = resolve_class(env, "java/util/Optional")?;
        let optional_of_nullable = env.get_static_method_id(
            &optional_class,
            "ofNullable",
            "(Ljava/lang/Object;)Ljava/util/Optional;",
        )?;

        Ok(JniCache {
            long_class,
            long_ctor,
            double_class,
            double_ctor,
            boolean_class,
            boolean_ctor,
            float_class,
            float_ctor,
            array_list_class,
            array_list_ctor,
            array_list_add,
            hash_map_class,
            hash_map_ctor,
            hash_map_put,
            optional_class,
            optional_of_nullable,
        })
    }

    /// Returns the initialized cache. Panics if called before `JNI_OnLoad`.
    #[inline]
    pub fn get() -> &'static JniCache {
        JNI_CACHE
            .get()
            .expect("JniCache used before JNI_OnLoad initialized it")
    }
}
