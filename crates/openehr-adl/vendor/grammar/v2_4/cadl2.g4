//
// description: Antlr4 grammar for cADL non-primitves sub-syntax of Archetype Definition Language (ADL2)
// author:      Thomas Beale <thomas.beale@openehr.org>
// contributors:Pieter Bos <pieter.bos@nedap.com>
// support:     openEHR Specifications PR tracker <https://openehr.atlassian.net/projects/SPECPR/issues>
// copyright:   Copyright (c) 2015 openEHR Foundation <http://www.openEHR.org>
// license:     Apache 2.0 License <http://www.apache.org/licenses/LICENSE-2.0.html>
//

grammar cadl2;
import cadl2_primitives, odin, base_expressions;

//
//  ======================= Top-level Objects ========================
//

c_complex_object: rm_type_id '[' ( ROOT_ID_CODE | ID_CODE ) ']' c_occurrences? ( SYM_MATCHES '{' ( c_attribute_def+ default_value? | default_value ) '}' )? ;

// ======================== Components =======================

c_objects: c_regular_object_ordered+ | c_inline_primitive_object ;

sibling_order: ( SYM_AFTER | SYM_BEFORE ) '[' ID_CODE ']' ;

c_regular_object_ordered: sibling_order? c_regular_object ;

c_regular_object:
      c_complex_object
    | c_archetype_root
    | c_complex_object_proxy
    | archetype_slot
    | c_regular_primitive_object
    ;

c_archetype_root: SYM_USE_ARCHETYPE rm_type_id '[' ID_CODE ',' archetype_ref ']' c_occurrences? ( SYM_MATCHES '{' c_attribute_def+ default_value? '}' )? ;

archetype_ref : ARCHETYPE_HRID | ARCHETYPE_REF ;

c_complex_object_proxy: SYM_USE_NODE rm_type_id '[' ID_CODE ']' c_occurrences? ADL_PATH ;

archetype_slot: SYM_ALLOW_ARCHETYPE rm_type_id '[' ID_CODE ']' (( c_occurrences? ( SYM_MATCHES '{' c_includes? c_excludes? '}' )? ) | SYM_CLOSED ) ;

c_attribute_def:
      c_attribute
    | c_attribute_tuple
    ;

// default value as an ODIN object block, optionally typed, e.g. _default = (DV_CODED_TEXT) <...>,
// or as a block in another syntax, e.g. _default = (json) <# ... #>
default_value: SYM_DEFAULT SYM_EQ ( object_value_block | INCLUDED_LANGUAGE_FRAGMENT ) ;

c_regular_primitive_object: rm_type_id '[' ID_CODE ']' c_occurrences? ( SYM_MATCHES '{' c_inline_primitive_object '}' )? ;

// We match regexes here, even though technically they are C_STRING instances. This is because the only
// workable solution to match a regex unambiguously appears to be to match with enclosing {}, which means
// as a C_OBJECT alternative, not as a C_STRING.
c_attribute: (ADL_PATH | rm_attribute_id) c_existence? c_cardinality? ( SYM_MATCHES ( '{' c_objects '}' | CONTAINED_REGEXP) )? ;

c_attribute_tuple : '[' rm_attribute_id ( ',' rm_attribute_id )* ']' SYM_MATCHES '{' c_primitive_tuple ( ',' c_primitive_tuple )* '}' ;

c_primitive_tuple : '[' c_primitive_tuple_item ( ',' c_primitive_tuple_item )* ']' ;

c_primitive_tuple_item: '{' c_inline_primitive_object '}' | CONTAINED_REGEXP;

c_includes : SYM_INCLUDE assertion+ ;
c_excludes : SYM_EXCLUDE assertion+ ;

c_existence: SYM_EXISTENCE SYM_MATCHES '{' existence '}' ;
existence: INTEGER | INTEGER '..' INTEGER ;

c_cardinality    : SYM_CARDINALITY SYM_MATCHES '{' cardinality '}' ;
cardinality      : multiplicity ( multiplicity_mod multiplicity_mod? )? ; // max of two
ordering_mod     : ';' ( SYM_ORDERED | SYM_UNORDERED ) ;
unique_mod       : ';' SYM_UNIQUE ;
multiplicity_mod : ordering_mod | unique_mod ;

c_occurrences : SYM_OCCURRENCES SYM_MATCHES '{' multiplicity '}' ;
multiplicity  : INTEGER | '*' | INTEGER '..' ( INTEGER | '*' ) ;

//
// ---------- Lexer patterns -----------------
//
