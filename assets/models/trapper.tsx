<?xml version='1.0' encoding='UTF-8'?>
<tileset version="1.10" tiledversion="1.10.2" name="trapper" tilewidth="48" tileheight="64" tilecount="128" columns="8">
 <properties>
  <property name="hitbox_width" type="float" value="1.0" />
  <property name="hitbox_height" type="float" value="2.25" />
 </properties>
 <image source="trapper.png" width="384" height="1024" />
 <tile id="0">
  <properties>
   <property name="action" value="idle" />
   <property name="dir" type="int" value="0" />
  </properties>
  <animation>
   <frame tileid="0" duration="1100" />
   <frame tileid="1" duration="900" />
  </animation>
 </tile>
 <tile id="8">
  <properties>
   <property name="action" value="idle" />
   <property name="dir" type="int" value="1" />
  </properties>
  <animation>
   <frame tileid="8" duration="1100" />
   <frame tileid="9" duration="900" />
  </animation>
 </tile>
 <tile id="16">
  <properties>
   <property name="action" value="idle" />
   <property name="dir" type="int" value="2" />
  </properties>
  <animation>
   <frame tileid="16" duration="1100" />
   <frame tileid="17" duration="900" />
  </animation>
 </tile>
 <tile id="24">
  <properties>
   <property name="action" value="idle" />
   <property name="dir" type="int" value="3" />
  </properties>
  <animation>
   <frame tileid="24" duration="1100" />
   <frame tileid="25" duration="900" />
  </animation>
 </tile>
 <tile id="32">
  <properties>
   <property name="action" value="idle" />
   <property name="dir" type="int" value="4" />
  </properties>
  <animation>
   <frame tileid="32" duration="1100" />
   <frame tileid="33" duration="900" />
  </animation>
 </tile>
 <tile id="40">
  <properties>
   <property name="action" value="idle" />
   <property name="dir" type="int" value="5" />
  </properties>
  <animation>
   <frame tileid="40" duration="1100" />
   <frame tileid="41" duration="900" />
  </animation>
 </tile>
 <tile id="48">
  <properties>
   <property name="action" value="idle" />
   <property name="dir" type="int" value="6" />
  </properties>
  <animation>
   <frame tileid="48" duration="1100" />
   <frame tileid="49" duration="900" />
  </animation>
 </tile>
 <tile id="56">
  <properties>
   <property name="action" value="idle" />
   <property name="dir" type="int" value="7" />
  </properties>
  <animation>
   <frame tileid="56" duration="1100" />
   <frame tileid="57" duration="900" />
  </animation>
 </tile>
 <tile id="64">
  <properties>
   <property name="action" value="walk" />
   <property name="dir" type="int" value="0" />
   <property name="step" type="bool" value="true" />
  </properties>
  <animation>
   <frame tileid="64" duration="125" />
   <frame tileid="65" duration="125" />
   <frame tileid="66" duration="125" />
   <frame tileid="67" duration="125" />
   <frame tileid="68" duration="125" />
   <frame tileid="69" duration="125" />
   <frame tileid="70" duration="125" />
   <frame tileid="71" duration="125" />
  </animation>
 </tile>
 <tile id="68">
  <properties>
   <property name="step" type="bool" value="true" />
  </properties>
 </tile>
 <tile id="72">
  <properties>
   <property name="action" value="walk" />
   <property name="dir" type="int" value="1" />
   <property name="step" type="bool" value="true" />
  </properties>
  <animation>
   <frame tileid="72" duration="125" />
   <frame tileid="73" duration="125" />
   <frame tileid="74" duration="125" />
   <frame tileid="75" duration="125" />
   <frame tileid="76" duration="125" />
   <frame tileid="77" duration="125" />
   <frame tileid="78" duration="125" />
   <frame tileid="79" duration="125" />
  </animation>
 </tile>
 <tile id="76">
  <properties>
   <property name="step" type="bool" value="true" />
  </properties>
 </tile>
 <tile id="80">
  <properties>
   <property name="action" value="walk" />
   <property name="dir" type="int" value="2" />
   <property name="step" type="bool" value="true" />
  </properties>
  <animation>
   <frame tileid="80" duration="125" />
   <frame tileid="81" duration="125" />
   <frame tileid="82" duration="125" />
   <frame tileid="83" duration="125" />
   <frame tileid="84" duration="125" />
   <frame tileid="85" duration="125" />
   <frame tileid="86" duration="125" />
   <frame tileid="87" duration="125" />
  </animation>
 </tile>
 <tile id="84">
  <properties>
   <property name="step" type="bool" value="true" />
  </properties>
 </tile>
 <tile id="88">
  <properties>
   <property name="action" value="walk" />
   <property name="dir" type="int" value="3" />
   <property name="step" type="bool" value="true" />
  </properties>
  <animation>
   <frame tileid="88" duration="125" />
   <frame tileid="89" duration="125" />
   <frame tileid="90" duration="125" />
   <frame tileid="91" duration="125" />
   <frame tileid="92" duration="125" />
   <frame tileid="93" duration="125" />
   <frame tileid="94" duration="125" />
   <frame tileid="95" duration="125" />
  </animation>
 </tile>
 <tile id="92">
  <properties>
   <property name="step" type="bool" value="true" />
  </properties>
 </tile>
 <tile id="96">
  <properties>
   <property name="action" value="walk" />
   <property name="dir" type="int" value="4" />
   <property name="step" type="bool" value="true" />
  </properties>
  <animation>
   <frame tileid="96" duration="125" />
   <frame tileid="97" duration="125" />
   <frame tileid="98" duration="125" />
   <frame tileid="99" duration="125" />
   <frame tileid="100" duration="125" />
   <frame tileid="101" duration="125" />
   <frame tileid="102" duration="125" />
   <frame tileid="103" duration="125" />
  </animation>
 </tile>
 <tile id="100">
  <properties>
   <property name="step" type="bool" value="true" />
  </properties>
 </tile>
 <tile id="104">
  <properties>
   <property name="action" value="walk" />
   <property name="dir" type="int" value="5" />
   <property name="step" type="bool" value="true" />
  </properties>
  <animation>
   <frame tileid="104" duration="125" />
   <frame tileid="105" duration="125" />
   <frame tileid="106" duration="125" />
   <frame tileid="107" duration="125" />
   <frame tileid="108" duration="125" />
   <frame tileid="109" duration="125" />
   <frame tileid="110" duration="125" />
   <frame tileid="111" duration="125" />
  </animation>
 </tile>
 <tile id="108">
  <properties>
   <property name="step" type="bool" value="true" />
  </properties>
 </tile>
 <tile id="112">
  <properties>
   <property name="action" value="walk" />
   <property name="dir" type="int" value="6" />
   <property name="step" type="bool" value="true" />
  </properties>
  <animation>
   <frame tileid="112" duration="125" />
   <frame tileid="113" duration="125" />
   <frame tileid="114" duration="125" />
   <frame tileid="115" duration="125" />
   <frame tileid="116" duration="125" />
   <frame tileid="117" duration="125" />
   <frame tileid="118" duration="125" />
   <frame tileid="119" duration="125" />
  </animation>
 </tile>
 <tile id="116">
  <properties>
   <property name="step" type="bool" value="true" />
  </properties>
 </tile>
 <tile id="120">
  <properties>
   <property name="action" value="walk" />
   <property name="dir" type="int" value="7" />
   <property name="step" type="bool" value="true" />
  </properties>
  <animation>
   <frame tileid="120" duration="125" />
   <frame tileid="121" duration="125" />
   <frame tileid="122" duration="125" />
   <frame tileid="123" duration="125" />
   <frame tileid="124" duration="125" />
   <frame tileid="125" duration="125" />
   <frame tileid="126" duration="125" />
   <frame tileid="127" duration="125" />
  </animation>
 </tile>
 <tile id="124">
  <properties>
   <property name="step" type="bool" value="true" />
  </properties>
 </tile>
</tileset>