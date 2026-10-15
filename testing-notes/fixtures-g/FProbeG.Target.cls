Class FProbeG.Target Extends %Persistent
{

/// The referenced method
ClassMethod TgtMethod(p1 As %Integer = 0) As %Integer
{
    Quit p1+1
}

/// decoy sibling, prefix collision on TgtMethod
ClassMethod TgtMethodX() As %Integer
{
    Quit 0
}

/// E6: self instance-qualified call site
Method MInst() As %Integer
{
    Quit ..TgtMethod(6)
}

/// E7: trigger body carries a call site
Trigger trg1 [ Event = INSERT ]
{
 write ##class(FProbeG.Target).TgtMethod(7),!
}

/// E8: property compute code carries a call site
Property P9 As %Integer [ SqlComputeCode = {Set {*} = ##class(FProbeG.Target).TgtMethod(8)}, SqlComputed];

Index idx1 On P9;

Query QAll() As %SQLQuery
{
 SELECT ID FROM FProbeG.Target
}

Method Edge8() As %Integer
{
    // E5: class name assembled from a string, eval'd
    set cls = "FProbeG"_".Target"
    set cmd = "s x=##class("_cls_").TgtMethod(8)"
    do $ZF(-4,1,cmd)
    Quit 0
}

}
