use ethers_core::types::{I256, U256};
use crate::{context::ContextBuilder, ExprRet};
use shared::{
    analyzer::AnalyzerLike,
    context::*,
    range::{
        elem::RangeOp,
        RangeEval,
        elem_ty::{
            Dynamic,
            Elem
        },
        SolcRange,
        Range,
    },
    nodes::{Concrete, Builtin, BuiltInNode, VarType},
    Edge, Node,
};

use solang_parser::pt::{Expression, Loc};

impl<T> BinOp for T where T: AnalyzerLike<Expr = Expression> + Sized {}
pub trait BinOp: AnalyzerLike<Expr = Expression> + Sized {
    /// Evaluate and execute a binary operation expression
    fn op_expr(
        &mut self,
        loc: Loc,
        lhs_expr: &Expression,
        rhs_expr: &Expression,
        ctx: ContextNode,
        op: RangeOp,
        assign: bool,
    ) -> ExprRet {
        let lhs_paths = self.parse_ctx_expr(lhs_expr, ctx);
        let rhs_paths = self.parse_ctx_expr(rhs_expr, ctx);
        match (lhs_paths, rhs_paths) {
            (ExprRet::Single((lhs_ctx, lhs)), ExprRet::Single((rhs_ctx, rhs))) => {
                let lhs_cvar = ContextVarNode::from(lhs).latest_version(self);
                let rhs_cvar = ContextVarNode::from(rhs).latest_version(self);
                let all_vars = self.op(loc, lhs_cvar, rhs_cvar, lhs_ctx, op, assign);
                if lhs_ctx != rhs_ctx {
                    ExprRet::Multi(vec![
                        all_vars,
                        self.op(loc, lhs_cvar, rhs_cvar, rhs_ctx, op, assign),
                    ])
                } else {
                    all_vars
                }
            }
            (ExprRet::Single((lhs_ctx, lhs)), ExprRet::Multi(rhs_sides)) => ExprRet::Multi(
                rhs_sides
                    .iter()
                    .map(|expr_ret| {
                        let (rhs_ctx, rhs) = expr_ret.expect_single();
                        let lhs_cvar = ContextVarNode::from(lhs).latest_version(self);
                        let rhs_cvar = ContextVarNode::from(rhs).latest_version(self);
                        let all_vars = self.op(loc, lhs_cvar, rhs_cvar, lhs_ctx, op, assign);
                        if lhs_ctx != rhs_ctx {
                            ExprRet::Multi(vec![
                                all_vars,
                                self.op(loc, lhs_cvar, rhs_cvar, rhs_ctx, op, assign),
                            ])
                        } else {
                            all_vars
                        }
                    })
                    .collect(),
            ),
            (ExprRet::Multi(lhs_sides), ExprRet::Single((rhs_ctx, rhs))) => ExprRet::Multi(
                lhs_sides
                    .iter()
                    .map(|expr_ret| {
                        let (lhs_ctx, lhs) = expr_ret.expect_single();
                        let lhs_cvar = ContextVarNode::from(lhs).latest_version(self);
                        let rhs_cvar = ContextVarNode::from(rhs).latest_version(self);
                        let all_vars = self.op(loc, lhs_cvar, rhs_cvar, lhs_ctx, op, assign);
                        if lhs_ctx != rhs_ctx {
                            ExprRet::Multi(vec![
                                all_vars,
                                self.op(loc, lhs_cvar, rhs_cvar, rhs_ctx, op, assign),
                            ])
                        } else {
                            all_vars
                        }
                    })
                    .collect(),
            ),
            (ExprRet::Multi(_lhs_sides), ExprRet::Multi(_rhs_sides)) => {
                todo!("here")
            }
            (_, ExprRet::CtxKilled) => ExprRet::CtxKilled,
            (ExprRet::CtxKilled, _) => ExprRet::CtxKilled,
            (_, _) => todo!(),
        }
    }

    /// Execute a binary operation after parsing the expressions
    fn op(
        &mut self,
        loc: Loc,
        lhs_cvar: ContextVarNode,
        rhs_cvar: ContextVarNode,
        ctx: ContextNode,
        op: RangeOp,
        assign: bool,
    ) -> ExprRet {
        // println!("op: {:?}", op);
        let new_lhs = if assign {
            self.advance_var_in_ctx(lhs_cvar, loc, ctx)
        } else {
            let mut new_lhs_underlying = ContextVar {
                loc: Some(loc),
                name: format!(
                    "tmp{}({} {} {})",
                    ctx.new_tmp(self),
                    lhs_cvar.name(self),
                    op.to_string(),
                    rhs_cvar.name(self)
                ),
                display_name: format!(
                    "({} {} {})",
                    lhs_cvar.display_name(self),
                    op.to_string(),
                    rhs_cvar.display_name(self)
                ),
                storage: None,
                is_tmp: true,
                is_symbolic: lhs_cvar.is_symbolic(self) || rhs_cvar.is_symbolic(self),
                tmp_of: Some(TmpConstruction::new(lhs_cvar, op, Some(rhs_cvar))),
                ty: lhs_cvar.underlying(self).ty.clone(),
            };

            // will potentially mutate the ty from concrete to builtin with a concrete range
            new_lhs_underlying.ty.concrete_to_builtin(self);

            let new_var = self.add_node(Node::ContextVar(new_lhs_underlying));
            self.add_edge(new_var, ctx, Edge::Context(ContextEdge::Variable));
            ContextVarNode::from(new_var)
        };

        let mut new_rhs = rhs_cvar;

        // TODO: change to only hit this path if !uncheck

        // TODO: If one of lhs_cvar OR rhs_cvar are not symbolic,
        // apply the requirement on the symbolic expression side instead of
        // ignoring the case where 
        if lhs_cvar.is_symbolic(self) && new_rhs.is_symbolic(self) {
            match op {
                RangeOp::Div | RangeOp::Mod => {
                    let tmp_rhs = self.advance_var_in_ctx(rhs_cvar, loc, ctx);
                    let zero_node = self.add_node(Node::Concrete(Concrete::from(U256::zero())));
                    let zero_node = self.add_node(Node::ContextVar(ContextVar::new_from_concrete(Loc::Implicit, zero_node.into(), self)));

                    let tmp_var = ContextVar {
                        loc: Some(loc),
                        name: format!(
                            "tmp{}({} != 0)",
                            ctx.new_tmp(self),
                            tmp_rhs.name(self),
                        ),
                        display_name: format!(
                            "({} != 0)",
                            tmp_rhs.display_name(self),
                        ),
                        storage: None,
                        is_tmp: true,
                        tmp_of: Some(TmpConstruction::new(new_lhs, RangeOp::Gt, Some(zero_node.into()))),
                        is_symbolic: true,
                        ty: VarType::BuiltIn(
                            BuiltInNode::from(self.builtin_or_add(Builtin::Bool)),
                            SolcRange::from(Concrete::Bool(true)),
                        ),
                    };

                    let cvar = ContextVarNode::from(self.add_node(Node::ContextVar(tmp_var)));
                    ctx.add_ctx_dep(cvar, self);

                    let range = tmp_rhs.range(self).expect("No range?");
                    if range.min_is_negative(self) {
                        let mut range_excls = range.range_exclusions();
                        let excl = Elem::from(Concrete::from(I256::zero()));
                        range_excls.push(SolcRange { min: excl.clone(), max: excl, exclusions: vec![] });
                        tmp_rhs.set_range_exclusions(self, range_excls);
                    } else {
                        // the new min is max(1, rhs.min)
                        let min = Elem::max(
                            tmp_rhs.range_min(self).expect("No range minimum?"),
                            Elem::from(Concrete::from(U256::from(1)))
                        );

                        tmp_rhs.set_range_min(self, min);
                        new_rhs = tmp_rhs;
                    }
                }
                RangeOp::Sub => {
                    let tmp_lhs = self.advance_var_in_ctx(lhs_cvar, loc, ctx);
                    // the new min is max(lhs.min, rhs.min)
                    let min = Elem::max(
                        tmp_lhs.range_min(self).expect("No range minimum?"),
                        Elem::Dynamic(Dynamic::new(rhs_cvar.into(), loc))
                    );
                    tmp_lhs.set_range_min(self, min);

                    let tmp_var = ContextVar {
                        loc: Some(loc),
                        name: format!(
                            "tmp{}({} >= {})",
                            ctx.new_tmp(self),
                            tmp_lhs.name(self),
                            new_rhs.name(self),
                        ),
                        display_name: format!(
                            "({} >= {})",
                            tmp_lhs.display_name(self),
                            new_rhs.display_name(self),
                        ),
                        storage: None,
                        is_tmp: true,
                        tmp_of: Some(TmpConstruction::new(tmp_lhs, RangeOp::Gte, Some(new_rhs))),
                        is_symbolic: true,
                        ty: VarType::BuiltIn(
                            BuiltInNode::from(self.builtin_or_add(Builtin::Bool)),
                            SolcRange::from(Concrete::Bool(true)),
                        ),
                    };

                    let cvar = ContextVarNode::from(self.add_node(Node::ContextVar(tmp_var)));
                    ctx.add_ctx_dep(cvar, self);
                }
                RangeOp::Add => {
                    let tmp_lhs = self.advance_var_in_ctx(lhs_cvar, loc, ctx);

                    // the new max is min(lhs.max, (2**256 - rhs.min))
                    let max = Elem::min(
                        tmp_lhs.range_max(self).expect("No range max?"),
                        Elem::from(Concrete::from(U256::MAX)) - Elem::Dynamic(Dynamic::new(rhs_cvar.into(), loc))
                    );

                    tmp_lhs.set_range_max(self, max);


                    let max_node = self.add_node(Node::Concrete(Concrete::from(U256::MAX)));
                    let max_node = self.add_node(Node::ContextVar(ContextVar::new_from_concrete(Loc::Implicit, max_node.into(), self)));

                    let (_, tmp_rhs) = self.op(loc, max_node.into(), new_rhs, ctx, RangeOp::Sub, false).expect_single();

                    let tmp_var = ContextVar {
                        loc: Some(loc),
                        name: format!(
                            "tmp{}({} <= 2**256 - 1 - {})",
                            ctx.new_tmp(self),
                            tmp_lhs.name(self),
                            new_rhs.name(self),
                        ),
                        display_name: format!(
                            "({} <= 2**256 - 1 - {})",
                            tmp_lhs.display_name(self),
                            new_rhs.display_name(self),
                        ),
                        storage: None,
                        is_tmp: true,
                        tmp_of: Some(TmpConstruction::new(tmp_lhs, RangeOp::Lte, Some(tmp_rhs.into()))),
                        is_symbolic: true,
                        ty: VarType::BuiltIn(
                            BuiltInNode::from(self.builtin_or_add(Builtin::Bool)),
                            SolcRange::from(Concrete::Bool(true)),
                        ),
                    };

                    let cvar = ContextVarNode::from(self.add_node(Node::ContextVar(tmp_var)));
                    ctx.add_ctx_dep(cvar, self);
                }
                RangeOp::Mul => {
                    let tmp_lhs = self.advance_var_in_ctx(lhs_cvar, loc, ctx);

                    // the new max is min(lhs.max, (2**256 / max(1, rhs.min)))
                    let max = Elem::min(
                        tmp_lhs.range_max(self).expect("No range max?"),
                        Elem::from(Concrete::from(U256::MAX)) / Elem::max(
                            Elem::from(Concrete::from(U256::from(1))),
                            Elem::Dynamic(Dynamic::new(rhs_cvar.into(), loc))
                        )
                    );

                    tmp_lhs.set_range_max(self, max);

                    let max_node = self.add_node(Node::Concrete(Concrete::from(U256::MAX)));
                    let max_node = self.add_node(Node::ContextVar(ContextVar::new_from_concrete(Loc::Implicit, max_node.into(), self)));

                    let (_, tmp_rhs) = self.op(loc, max_node.into(), new_rhs, ctx, RangeOp::Div, false).expect_single();

                    let tmp_var = ContextVar {
                        loc: Some(loc),
                        name: format!(
                            "tmp{}({} <= 2**256 - 1 / {})",
                            ctx.new_tmp(self),
                            tmp_lhs.name(self),
                            new_rhs.name(self),
                        ),
                        display_name: format!(
                            "({} <= 2**256 - 1 / {})",
                            tmp_lhs.display_name(self),
                            new_rhs.display_name(self),
                        ),
                        storage: None,
                        is_tmp: true,
                        tmp_of: Some(TmpConstruction::new(tmp_lhs, RangeOp::Lte, Some(tmp_rhs.into()))),
                        is_symbolic: true,
                        ty: VarType::BuiltIn(
                            BuiltInNode::from(self.builtin_or_add(Builtin::Bool)),
                            SolcRange::from(Concrete::Bool(true)),
                        ),
                    };

                    let cvar = ContextVarNode::from(self.add_node(Node::ContextVar(tmp_var)));
                    ctx.add_ctx_dep(cvar, self);
                }
                _ => {}
            };
        }

        let lhs_range = if let Some(lhs_range) = new_lhs.range(self) {
            lhs_range
        } else {
            new_rhs
                .range(self)
                .expect("Neither lhs nor rhs had a usable range")
        };

        let func = SolcRange::dyn_fn_from_op(op);
        let new_range = func(lhs_range, new_rhs, loc);
        new_lhs.set_range_min(self, new_range.range_min());
        new_lhs.set_range_max(self, new_range.range_max());

        // last ditch effort to prevent exponentiation from having a minimum of 1 instead of 0.
        // if the lhs is 0 check if the rhs is also 0, otherwise set minimum to 0.
        if matches!(op, RangeOp::Exp) {
            if let (Some(old_lhs_range), Some(rhs_range)) = (lhs_cvar.range(self), new_rhs.range(self)) {
                let zero = Elem::from(Concrete::from(U256::zero()));
                let zero_range = SolcRange { min: zero.clone(), max: zero.clone(), exclusions: vec![] };
                // We have to check if the the lhs and the right hand side contain the zero range.
                // If they both do, we have to set the minimum to zero due to 0**0 = 1, but 0**x = 0.
                // This is technically a slight widening of the interval and could be improved.
                if old_lhs_range.contains(&zero_range, self) && rhs_range.contains(&zero_range, self) {
                    new_lhs.set_range_min(self, zero);
                }
            }
        }
        ExprRet::Single((ctx, new_lhs.into()))
    }
}
