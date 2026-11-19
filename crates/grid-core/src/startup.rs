//! Starting Grid with the OS, as chosen in Settings → General, and how that maps onto what the OS
//! keeps: per-user and all-users start entries, administrator tasks and a per-user opt-out (on
//! Windows, Run keys, Task Scheduler tasks and a registry value). Platform layers read and write
//! the `Entries`; this works out which.

/// The start-with-the-OS checkboxes:
///
/// ```text
/// [ ] Start Grid when Windows starts        me
///       [ ] For all users on this PC         all_users
///       [ ] As administrator                 admin
///             [ ] For all administrators     all_admins
/// ```
///
/// `me` is about this user only: unticking it while Grid starts for all users (or all
/// administrators) skips it for this user and leaves it for everyone else. The machine-wide
/// options change only when their own boxes do, so a box someone can't see (the administrator
/// ones, for a standard account) is never turned off by mistake.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Startup {
    /// Grid starts when this user signs in.
    pub me: bool,
    /// Grid starts when anyone signs in.
    pub all_users: bool,
    /// This user's Grid starts as administrator.
    pub admin: bool,
    /// Grid starts as administrator for every administrator account.
    pub all_admins: bool,
}

/// What the OS keeps to start Grid.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Entries {
    /// This user's start entry (no administrator rights needed).
    pub user: bool,
    /// The start entry for all users.
    pub all_users: bool,
    /// This user's administrator task.
    pub user_task: bool,
    /// The administrator task for all administrators.
    pub admins_task: bool,
    /// This user skips the all-users entry and the all-administrators task (no administrator
    /// rights needed).
    pub opt_out: bool,
}

impl Startup {
    /// The entries that make Grid start this way. `user_is_admin`: whether this user's account is
    /// an administrator (the all-administrators task starts Grid only for those). Never two that
    /// start Grid for the same user where avoidable: an administrator task replaces this user's
    /// own entry. (The all-users entry can still overlap an administrator task; the normal copy it
    /// starts then quits, see `Entries::quits_at_sign_in`.)
    pub fn entries(self, user_is_admin: bool) -> Entries {
        let admins_mine = self.all_admins && user_is_admin;
        let user_task = self.me && self.admin && !admins_mine;
        Entries {
            user: self.me && !self.all_users && !user_task && !admins_mine,
            all_users: self.all_users,
            user_task,
            admins_task: self.all_admins,
            opt_out: !self.me && (self.all_users || admins_mine),
        }
    }

    /// The checkboxes as the OS's entries leave them (see `entries`).
    pub fn from_entries(e: Entries, user_is_admin: bool) -> Startup {
        let admin = e.user_task || (e.admins_task && user_is_admin);
        Startup {
            me: !e.opt_out && (e.user || e.all_users || admin),
            all_users: e.all_users,
            admin,
            all_admins: e.admins_task,
        }
    }
}

impl Entries {
    /// Whether a Grid started at sign-in should quit at once: this user opted out, or (for a
    /// normal, not administrator, copy) an administrator task starts one for them too.
    pub fn quits_at_sign_in(self, elevated: bool, user_is_admin: bool) -> bool {
        self.opt_out || (!elevated && (self.user_task || (self.admins_task && user_is_admin)))
    }

    /// Whether changing from `self` to `to` changes anything that needs administrator rights.
    pub fn needs_admin(self, to: Entries) -> bool {
        self.all_users != to.all_users || self.user_task != to.user_task || self.admins_task != to.admins_task
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const ME: Startup = Startup { me: true, all_users: false, admin: false, all_admins: false };
    const NONE: Entries = Entries { user: false, all_users: false, user_task: false, admins_task: false, opt_out: false };

    #[test]
    fn one_entry_per_way_to_start() {
        assert_eq!(Startup::default().entries(true), NONE);
        assert_eq!(ME.entries(true), Entries { user: true, ..NONE });
        assert_eq!(Startup { all_users: true, ..ME }.entries(true), Entries { all_users: true, ..NONE });
        assert_eq!(Startup { admin: true, ..ME }.entries(true), Entries { user_task: true, ..NONE });
        let admins = Startup { admin: true, all_admins: true, ..ME };
        assert_eq!(admins.entries(true), Entries { admins_task: true, ..NONE });
    }

    #[test]
    fn reads_back_what_it_wrote() {
        for me in [false, true] {
            for all_users in [false, true] {
                for (admin, all_admins) in [(false, false), (true, false), (true, true)] {
                    let s = Startup { me, all_users, admin: admin && (me || all_admins), all_admins };
                    assert_eq!(Startup::from_entries(s.entries(true), true), s, "{s:?}");
                }
            }
        }
        assert_eq!(Startup::from_entries(NONE, true), Startup::default());
    }

    #[test]
    fn unticking_me_only_skips_this_user() {
        let s = Startup { me: false, all_users: true, ..Startup::default() };
        let e = s.entries(false);
        assert!(e.all_users && e.opt_out && !e.user);
        assert!(e.quits_at_sign_in(false, false));
        // Ticking it again starts Grid for this user, with the all-users entry kept.
        let back = Startup { me: true, ..s }.entries(false);
        assert!(back.all_users && !back.opt_out);
        assert!(!back.needs_admin(e) && !e.needs_admin(back));
    }

    #[test]
    fn standard_users_keep_the_administrators_task() {
        // A standard account doesn't see the administrator boxes; whatever it changes, the
        // all-administrators task stays.
        let e = Entries { admins_task: true, ..NONE };
        let shown = Startup::from_entries(e, false);
        assert_eq!(shown, Startup { all_admins: true, ..Startup::default() });
        for changed in [Startup { me: true, ..shown }, Startup { me: true, all_users: true, ..shown }] {
            assert!(changed.entries(false).admins_task);
            // The task doesn't start Grid for them, so they get their own entry.
            assert_eq!(changed.entries(false).user, !changed.all_users);
        }
    }

    #[test]
    fn administrators_task_only_counts_for_administrators() {
        let e = Entries { admins_task: true, ..NONE };
        assert!(!Startup::from_entries(e, false).me);
        assert!(Startup::from_entries(e, true).me && Startup::from_entries(e, true).admin);
        assert!(!e.quits_at_sign_in(false, false));
        assert!(e.quits_at_sign_in(false, true));
        assert!(!e.quits_at_sign_in(true, true));
    }

    #[test]
    fn normal_copy_quits_for_an_administrator_task() {
        let e = Startup { all_users: true, admin: true, ..ME }.entries(true);
        assert!(e.all_users && e.user_task);
        assert!(e.quits_at_sign_in(false, true));
        assert!(!e.quits_at_sign_in(true, true));
        assert!(!Startup { all_users: true, ..ME }.entries(true).quits_at_sign_in(false, true));
    }

    #[test]
    fn opted_out_users_skip_even_the_administrator_copy() {
        let e = Startup { admin: true, all_admins: true, ..Startup::default() }.entries(true);
        assert!(e.opt_out && e.admins_task);
        assert!(e.quits_at_sign_in(true, true));
    }

    #[test]
    fn only_machine_wide_changes_need_administrator_rights() {
        assert!(!NONE.needs_admin(ME.entries(true)));
        assert!(NONE.needs_admin(Startup { all_users: true, ..ME }.entries(true)));
        assert!(NONE.needs_admin(Startup { admin: true, ..ME }.entries(true)));
    }
}
