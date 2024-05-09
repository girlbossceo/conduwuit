#[macro_export]
macro_rules! mod_ctor {
	( $($body:block)? ) => {
		$crate::mod_init! {{
			$crate::debug_info!("Module loaded");
			$($body)?
		}}
	}
}

#[macro_export]
macro_rules! mod_dtor {
	( $($body:block)? ) => {
		$crate::mod_fini! {{
			$crate::debug_info!("Module unloading");
			$($body)?
			$crate::mods::canary::report();
		}}
	}
}

#[macro_export]
macro_rules! mod_init {
	($body:block) => {
		#[used]
		#[cfg_attr(target_family = "unix", link_section = ".init_array")]
		static MOD_INIT: extern "C" fn() = { _mod_init };

		#[cfg_attr(target_family = "unix", link_section = ".text.startup")]
		extern "C" fn _mod_init() -> () $body
	};
}

#[macro_export]
macro_rules! mod_fini {
	($body:block) => {
		#[used]
		#[cfg_attr(target_family = "unix", link_section = ".fini_array")]
		static MOD_FINI: extern "C" fn() = { _mod_fini };

		#[cfg_attr(target_family = "unix", link_section = ".text.startup")]
		extern "C" fn _mod_fini() -> () $body
	};
}
